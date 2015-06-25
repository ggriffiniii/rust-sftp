#![crate_name = "sftp"]
#![feature(convert)]
#![feature(associated_consts)]
#![feature(collections)]
#![feature(clone_from_slice)]

extern crate byteorder;

mod packets;
mod error;

use std::io;
use error::Result;
use byteorder::{WriteBytesExt, BigEndian};
use packets::Sendable;
use std::io::Write;
use packets::Request;
use std::thread;
use std::sync::{Arc, Mutex, MutexGuard, atomic};
use std::collections::HashMap;
use std::sync::mpsc;

pub use packets::FileAttr;

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
                    tx.send(Ok(resp.packet)).unwrap();
                },
                None => { Self::broadcast_error(&mut state, error::Error::NoMatchingRequest(resp.req_id)); return; },
            }
        }
    }

    fn broadcast_error(state: &mut MutexGuard<ReceiverState>, e: error::Error) {
        let arc_wrapped = Arc::new(Box::new(e));
        for (_, tx) in state.requests.iter() {
            tx.send(Err(error::Error::ReceiverDisconnected(arc_wrapped.clone()))).unwrap();
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
        //writeln!(&mut io::stderr(), "Send Request: {:?}", *packet);
        Ok(rx)
    }

    fn send_receive<P : packets::Request>(&self, packet : &P) ->
        Result<packets::SftpResponsePacket> {
            let rx = try!(self.send(packet));
            let resp = rx.recv().unwrap();
            //writeln!(&mut io::stderr(), "Received Response: {:?}", resp);
            resp
    }
}

pub struct Client<W> {
    sender: Arc<ClientSender<W>>,
}

impl<W> Client<W> where W : 'static + io::Write + Send {
	pub fn new<R>(mut r: R, w: W) -> Result<Client<W>> where R : 'static + io::Read + Send {
        let s = ClientSender{
            w: Mutex::new(w),
            recv_state: Arc::new(Mutex::new(ReceiverState{requests: HashMap::new(), recv_error: None})),
            req_id: atomic::AtomicUsize::new(0),
        };
        try!(s.send_init());
        let resp = try!(packets::recv(&mut r));
        //writeln!(&mut io::stderr(), "Received Response: {:?}", resp);
        match resp.packet {
            packets::SftpResponsePacket::Version(x) => {
                if x.version != 3 {
                    return Err(error::Error::MismatchedVersion(x.version));
                }
            },
            x => return Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
        let r = ClientReceiver{
            r: Mutex::new(r),
            state: s.recv_state.clone(),
        };
        thread::spawn(move || r.recv());
        Ok(Client{sender: Arc::new(s)})
	}

    pub fn stat(&mut self, path: String) -> Result<packets::FileAttr> {
        let p = packets::FxpStat{path: path.into_bytes()};
        self.do_stat(p)
    }

    pub fn lstat(&mut self, path: String) -> Result<packets::FileAttr> {
        let p = packets::FxpLStat{path: path.into_bytes()};
        self.do_stat(p)
    }

    fn do_stat<T : packets::Request>(&mut self, p: T) -> Result<packets::FileAttr> {
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Attrs(attrs) => Ok(attrs),
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
            x => Err(error::Error::UnexpectedResponse(Box::new(x)))
        }
    }

    pub fn setstat(&mut self, path: String, attrs: packets::FileAttr) -> Result<()> {
        let p = packets::FxpSetStat{path: path.into_bytes(), attrs: attrs};
        let resp = try!(self.sender.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }

    pub fn mkdir(&mut self, path: String) -> Result<()> {
        let p = packets::FxpMkDir{path: path.into_bytes(), attrs: packets::FileAttr::new()};
        let resp = try!(self.sender.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }

    pub fn rmdir(&mut self, path: String) -> Result<()> {
        let p = packets::FxpRmDir{path: path.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }

    pub fn realpath(&mut self, path: String) -> Result<packets::Name> {
        let p = packets::FxpRealPath{path: path.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Name(mut name) => {
                if let Some(name) = name.names.pop() {
                    Ok(name)
                } else {
                    Err(error::Error::UnexpectedResponse(Box::new(packets::SftpResponsePacket::Name(name))))
                }
            },
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
            x => Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
    }

    pub fn rename(&mut self, oldpath: String, newpath: String) -> Result<()> {
        let p = packets::FxpRename{oldpath: oldpath.into_bytes(), newpath: newpath.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }

    pub fn readlink(&mut self, path: String) -> Result<packets::Name> {
        let p = packets::FxpReadLink{path: path.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Name(mut name) => {
                if let Some(name) = name.names.pop() {
                    Ok(name)
                } else {
                    Err(error::Error::UnexpectedResponse(Box::new(packets::SftpResponsePacket::Name(name))))
                }
            },
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
            x => Err(error::Error::UnexpectedResponse(Box::new(x))),
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
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
            x => Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
    }

    pub fn remove(&mut self, filename: String) -> Result<()> {
        let p = packets::FxpRemove{filename: filename.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }

    pub fn readdir(&mut self, path: String) -> Result<ReadDir<W>> {
        let p = packets::FxpOpenDir{path: path.into_bytes()};
        let resp = try!(self.sender.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Handle(handle) => {
                Ok(ReadDir{client: self.sender.clone(), handle: handle.handle, names: Vec::new().into_iter()})
            },
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
            x => Err(error::Error::UnexpectedResponse(Box::new(x))),
        }
    }

    fn expect_status_response(resp : packets::SftpResponsePacket) -> Result<()> {
        match resp {
            packets::SftpResponsePacket::Status(packets::FxpStatus{code:
                packets::FxpStatusCode::Ok, msg: _}) => Ok(()),
            packets::SftpResponsePacket::Status(status) => Err(error::Error::FromServer(Box::new(status))),
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

pub struct File<W> where W : 'static + io::Write + Send {
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

    pub fn setstat(&mut self, attrs: packets::FileAttr) -> Result<()> {
        let p = packets::FxpFSetStat{handle: self.handle.clone(), attrs: attrs};
        let resp = try!(self.client.send_receive(&p));
        Client::<W>::expect_status_response(resp)
    }
}

impl<W> Drop for File<W> where W : 'static + io::Write + Send {
    fn drop(&mut self) {
        let p = packets::FxpClose{handle: self.handle.clone()};
        let _ = self.client.send_receive(&p);
    }
}

impl<W> io::Read for File<W> where W : 'static + io::Write + Send {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let p = packets::FxpRead{handle: self.handle.clone(),
                                 offset: self.offset,
                                 len: buf.len() as u32};
        let resp = match self.client.send_receive(&p) {
            Ok(data) => data,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "unknown error")),
        };
        match resp {
            packets::SftpResponsePacket::Data(mut data) => {
                let n = buf.clone_from_slice(&mut data.data[..]);
                self.offset += n as u64;
                Ok(n)
            },
            packets::SftpResponsePacket::Status(packets::FxpStatus{code: packets::FxpStatusCode::EOF, msg: _}) => Ok(0),
            packets::SftpResponsePacket::Status(status) => Err(From::from(status)),
            _ => Err(io::Error::new(io::ErrorKind::Other, "unknown error")),
        }
    }
}

impl<W> io::Write for File<W> where W : 'static + io::Write + Send {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let p = packets::FxpWrite{handle: self.handle.clone(),
                                  offset: self.offset,
                                  data: buf.into()};
        let resp = match self.client.send_receive(&p) {
            Ok(data) => data,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "unknown error")),
        };
        match resp {
            packets::SftpResponsePacket::Status(packets::FxpStatus{code: packets::FxpStatusCode::Ok, msg: _}) => { self.offset += p.data.len() as u64; Ok(p.data.len()) },
            packets::SftpResponsePacket::Status(status) => Err(From::from(status)),
            _ => Err(io::Error::new(io::ErrorKind::Other, "unknown error")),
        }
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl<W> io::Seek for File<W> where W : 'static + io::Write + Send {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let soffset = self.offset as i64;
        self.offset = match pos {
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

pub struct ReadDir<W> where W : 'static + io::Write + Send {
    client: Arc<ClientSender<W>>,
    handle: Vec<u8>,
    names: std::vec::IntoIter<packets::Name>,
}

impl<W> Drop for ReadDir<W> where W : 'static + io::Write + Send {
    fn drop(&mut self) {
        let p = packets::FxpClose{handle: self.handle.clone()};
        let _ = self.client.send_receive(&p);
    }
}

impl<W> Iterator for ReadDir<W> where W : 'static + io::Write + Send {
    type Item = Result<packets::Name>;

    fn next(&mut self) -> Option<Result<packets::Name>> {
        match self.names.next() {
            Some(name) => Some(Ok(name)),
            None => {
                let p = packets::FxpReadDir{handle: self.handle.clone()};
                let resp = match self.client.send_receive(&p) {
                    Ok(x) => x,
                    Err(x) => return Some(Err(x)),
                };
                match resp {
                    packets::SftpResponsePacket::Status(packets::FxpStatus{code: packets::FxpStatusCode::EOF, msg: _}) => {
                        None
                    },
                    packets::SftpResponsePacket::Name(names) => {
                        self.names = names.names.into_iter();
                        self.next()
                    },
                    x => Some(Err(error::Error::UnexpectedResponse(Box::new(x)))),
                }
            },
        }
    }
}
