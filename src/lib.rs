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
use std::sync::{Arc, Mutex, atomic};
use std::collections::HashMap;
use std::sync::mpsc;

type ReqId = u32;
type ReqMap = HashMap<ReqId, mpsc::Sender<Result<packets::SftpResponsePacket>>>;

struct ReceiverState {
    requests: ReqMap,
    prev_error: bool,
}

struct ClientReceiver<R> {
    r: Mutex<R>,
    state: Arc<Mutex<ReceiverState>>,
}

impl<R> ClientReceiver<R> where R : 'static + io::Read + Send {
    fn recv(&self) -> Result<()> {
        let mut r = self.r.lock().unwrap();
        loop {
            let resp = match packets::recv(&mut *r) {
                Err(x) => {
                    self.broadcastErr();
                    return Err(std::convert::From::from(x));
                },
                Ok(x) => x,
            };
            let mut state = self.state.lock().unwrap();
            match state.requests.remove(&resp.req_id) {
                Some(tx) => {
                    tx.send(Ok(resp.packet));
                },
                None => {
                    for (_, tx) in state.requests.iter() {
                        tx.send(Err(error::Error::NoMatchingRequest(resp.req_id)));
                    }
                    state.requests.clear();
                    return Err(error::Error::NoMatchingRequest(resp.req_id));
                },
            }
        }
        Ok(())
    }

    fn broadcastErr(&self) {
        let mut state = self.state.lock().unwrap();
        state.prev_error = true;
        for (_, tx) in state.requests.iter() {
            //TODO: send better error
            tx.send(Err(error::Error::PreviousError));
        }
        state.requests.clear();
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
            if recv_state.prev_error {
                return Err(error::Error::PreviousError);
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
            recv_state: Arc::new(Mutex::new(ReceiverState{requests: HashMap::new(), prev_error: false})),
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
            x => return Err(error::Error::UnexpectedResponse(x)),
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
            x => Err(error::Error::UnexpectedResponse(x))
        }
    }

    pub fn open(&mut self, path: String) -> Result<File<W>> {
        Ok(File{client: self.sender.clone(), handle: Vec::<u8>::new(), offset: 0})
    }
}

pub struct File<W> {
    client: Arc<ClientSender<W>>,
    handle: Vec<u8>,
    offset: u64,
}

impl<W> File<W>  where W : 'static + io::Write + Send {
    fn stat(&mut self) -> Result<packets::FileAttr> {
        let p = packets::FxpFStat{handle: self.handle.clone()};
        let resp = try!(self.client.send_receive(&p));
        match resp {
            packets::SftpResponsePacket::Attrs(attrs) => {
                Ok(attrs)
            },
            x => Err(error::Error::UnexpectedResponse(x))
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
