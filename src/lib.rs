#![crate_name = "sftp"]
#![feature(convert)]
#![feature(associated_consts)]

extern crate byteorder;

mod packets;
mod error;

use std::io;
use error::Result;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use packets::Sendable;
use std::io::Write;
use packets::Request;
use std::thread;
use std::sync::{Arc, Mutex, MutexGuard, atomic};
use std::collections::HashMap;
use std::sync::mpsc;

type ReqId = u32;
type ReqMap = HashMap<ReqId, mpsc::Sender<Result<packets::SftpResponsePacket>>>;

struct ReceiverState {
    requests: ReqMap,
    recv_error: Option<Arc<Box<error::Error>>>,
}

struct ClientReceiver<R> {
    r: Mutex<R>,
    state: Arc<Mutex<ReceiverState>>,
}

impl<R> ClientReceiver<R> where R : 'static + io::Read + Send {
    fn recv(&self) {
        let mut r = self.r.lock().unwrap();
        loop {
            let resp = match packets::recv(&mut *r) {
                Err(e) => { Self::broadcast_error(&mut self.state.lock().unwrap(), e); return; },
                Ok(x) => x,
            };
            let mut state = self.state.lock().unwrap();
            match state.requests.remove(&resp.req_id) {
                Some(tx) => {
                    tx.send(Ok(resp.packet));
                },
                None => { Self::broadcast_error(&mut state, error::Error::NoMatchingRequest(resp.req_id)); return; },
            }
        }
    }

    fn broadcast_error(state: &mut MutexGuard<ReceiverState>, e: error::Error) {
        let arc_wrapped = Arc::new(Box::new(e));
        for (_, tx) in state.requests.iter() {
            tx.send(Err(error::Error::ReceiverDisconnected(arc_wrapped.clone())));
        }
        state.requests.clear();
        state.recv_error = Some(arc_wrapped.clone());
    }
}

struct ClientSender<W> {
    w: Mutex<W>,
    recv_state: Arc<Mutex<ReceiverState>>,
    req_id: atomic::AtomicUsize,
}

impl<W> ClientSender<W> where W : 'static + io::Write + Send {
    fn next_id(&self) -> ReqId {
        self.req_id.fetch_add(1, atomic::Ordering::Relaxed) as ReqId
    }

    fn send_init(&self) -> Result<usize> {
        let mut n : usize = 0;
        let mut bytes : Vec<u8> = Vec::new();
        let init_packet = packets::FxpInit{version: 3, extensions: Vec::new()};
        try!(init_packet.write_to(&mut bytes));
        let mut w = self.w.lock().unwrap();
        try!(w.write_u32::<BigEndian>(bytes.len() as u32 + 1));
        n += 4;
        try!(w.write_all(&[packets::FxpInit::msg_type()][..]));
        n += 1;
        try!(w.write_all(bytes.as_slice()));
        n += bytes.len();
        Ok(n)
    }

    fn send<P : packets::Request>(&self, packet : &P) -> Result<mpsc::Receiver<Result<packets::SftpResponsePacket>>> {
        let req_id = self.next_id();
        let (tx, rx) = mpsc::channel();
        {
            let mut recv_state = self.recv_state.lock().unwrap();
            if let Some(ref e) = recv_state.recv_error {
                return Err(error::Error::ReceiverDisconnected(e.clone()));
            }
            recv_state.requests.insert(req_id, tx);
        }
        let mut bytes : Vec<u8> = Vec::new();
        try!(packet.write_to(&mut bytes));
        let mut w = self.w.lock().unwrap();
        try!(w.write_u32::<BigEndian>(bytes.len() as u32 + 5));
        try!(w.write_all(&[P::msg_type()][..]));
        try!(w.write_u32::<BigEndian>(req_id));
        try!(w.write_all(bytes.as_slice()));
        Ok(rx)
    }

    fn send_receive<P : packets::Request>(&self, packet : &P) ->
        Result<packets::SftpResponsePacket> {
            let rx = try!(self.send(packet));
            let resp = rx.recv().unwrap();
            writeln!(&mut io::stderr(), "Received Response: {:?}", resp);
            resp
    }
}

pub struct Client<W> {
    sender: Arc<ClientSender<W>>,
}

impl<W> Client<W> where W : 'static + io::Write + Send {
	pub fn new<R>(mut r: R, mut w: W) -> Result<Client<W>> where R : 'static + io::Read + Send {
        let mut s = ClientSender{
            w: Mutex::new(w),
            recv_state: Arc::new(Mutex::new(ReceiverState{requests: HashMap::new(), recv_error: None})),
            req_id: atomic::AtomicUsize::new(0),
        };
        try!(s.send_init());
        let resp = try!(packets::recv(&mut r));
        writeln!(&mut io::stderr(), "Received Response: {:?}", resp);
        match resp.packet {
            packets::SftpResponsePacket::Version(x) => {
                if x.version != 3 {
                    return Err(error::Error::MismatchedVersion(x.version));
                }
            },
            x => return Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
        let mut r = ClientReceiver{
            r: Mutex::new(r),
            state: s.recv_state.clone(),
        };
        thread::spawn(move || r.recv());
        Ok(Client{sender: Arc::new(s)})
	}

    pub fn stat(&mut self, path: String) -> Result<packets::FileAttr> {
        let p = packets::FxpStat{path: path.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Attrs(attrs) => {
                Ok(attrs)
            },
            x => Err(error::Error::UnexpectedResponse(Box::new(x)))
        }
    }

    pub fn open_options(&mut self) -> OpenOptions<W> {
        OpenOptions{client: self, flags: 0}
    }

    fn open(&mut self, filename: String, pflags: u32) -> Result<File<W>> {
        let p = packets::FxpOpen{
            filename: filename.into_bytes(),
            pflags: pflags,
            attrs: packets::FileAttr::new(),
        };
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Handle(handle) => {
                Ok(File{client: self.sender.clone(), handle: handle.handle, offset: 0})
            },
            x => Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
    }
}

const SSH_FXF_READ : u32 = 0x00000001;
const SSH_FXF_WRITE : u32 = 0x00000002;
const SSH_FXF_APPEND : u32 = 0x00000004;
const SSH_FXF_CREAT : u32 = 0x00000008;
const SSH_FXF_TRUNC : u32 = 0x00000010;
const SSH_FXF_EXCL : u32 = 0x00000020;

pub struct OpenOptions<'a, W> where W: 'a {
    client: &'a mut Client<W>,
    flags: u32,
}

impl<'a, W> OpenOptions<'a, W> where W : 'static + io::Write + Send {
    fn flag(&mut self, bit: u32, enabled: bool) -> &mut OpenOptions<'a, W> {
        if enabled {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
        self
    }

    pub fn read(&mut self, read: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_READ, read)
    }

    pub fn write(&mut self, write: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_WRITE, write)
    }

    pub fn append(&mut self, append: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_APPEND, append)
    }

    pub fn create(&mut self, create: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_CREAT, create)
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_TRUNC, truncate)
    }

    pub fn exclude(&mut self, exclude: bool) -> &mut OpenOptions<'a, W> {
        self.flag(SSH_FXF_EXCL, exclude)
    }

    pub fn open(&mut self, path: String) -> Result<File<W>> {
        self.client.open(path, self.flags)
    }
}

pub struct File<W> {
    client: Arc<ClientSender<W>>,
    handle: Vec<u8>,
    offset: u64,
}

impl<W> File<W>  where W : 'static + io::Write + Send {
    pub fn stat(&mut self) -> Result<packets::FileAttr> {
        let p = packets::FxpFStat{handle: self.handle.clone()};
        let resp = try!(self.client.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Attrs(attrs) => {
                Ok(attrs)
            },
            x => Err(error::Error::UnexpectedResponse(Box::new(x)))
        }
    }
}

impl<W> io::Seek for File<W> where W : 'static + io::Write + Send {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let soffset = self.offset as i64;
        self.offset = match pos {
            io::SeekFrom::Start(i) if i < 0 => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "can not seek past beginning of file"));
            },
            io::SeekFrom::Start(i) => i,
            io::SeekFrom::Current(i) if soffset + i < 0 => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "can not seek past beginning of file"));
            },
            io::SeekFrom::Current(i) => (soffset + i) as u64,
            io::SeekFrom::End(i) => {
                let attr = match self.stat() {
                    Ok(attr) => attr,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "unknown error")),
                };
                match attr.size {
                    Some(size) => {
                        if (size as i64) + i < 0 {
                            return Err(io::Error::new(io::ErrorKind::InvalidInput, "can not seek past beginning of file"));
                        }
                        size + i as u64
                    },
                    None => return Err(io::Error::new(io::ErrorKind::Other, "size not known")),
                }
            }
        };
        Ok(self.offset)
    }
}
