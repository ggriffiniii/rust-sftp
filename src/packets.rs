extern crate byteorder;

use self::byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};

use std::io;
use std::io::Read;
use error::{Error, Result};

// Init
const SSH_FXP_INIT : u8 = 1;
const SSH_FXP_VERSION : u8 = 2;

// Requests
const SSH_FXP_OPEN : u8 = 3;
//const SSH_FXP_CLOSE : u8 = 4;
//const SSH_FXP_READ : u8 = 5;
//const SSH_FXP_WRITE : u8 = 6;
//const SSH_FXP_LSTAT : u8 = 7;
const SSH_FXP_FSTAT : u8 = 8;
//const SSH_FXP_SETSTAT : u8 = 9;
//const SSH_FXP_FSETSTAT : u8 = 10;
//const SSH_FXP_OPENDIR : u8 = 11;
//const SSH_FXP_READDIR : u8 = 12;
//const SSH_FXP_REMOVE : u8 = 13;
//const SSH_FXP_MKDIR : u8 = 14;
//const SSH_FXP_RMDIR : u8 = 15;
//const SSH_FXP_REALPATH : u8 = 16;
const SSH_FXP_STAT : u8 = 17;
//const SSH_FXP_RENAME : u8 = 18;
//const SSH_FXP_READLINK : u8 = 19;
//const SSH_FXP_SYMLINK : u8 = 20;

// Responses
const SSH_FXP_STATUS : u8 = 101;
const SSH_FXP_HANDLE : u8 = 102;
//const SSH_FXP_DATA : u8 = 103;
//const SSH_FXP_NAME : u8 = 104;
const SSH_FXP_ATTRS : u8 = 105;
//const SSH_FXP_EXTENDED : u8 = 200;
//const SSH_FXP_EXTENDED_REPLY : u8 = 201;

pub trait Request : Sendable {
    fn msg_type() -> u8;
}

pub trait Sendable {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize>;
}

pub trait Response : Receivable {
    fn msg_type() -> u8;
}

pub trait Receivable {
    fn recv<R : io::Read>(r: &mut io::Take<R>) -> Result<Self>;
}

#[derive(Debug)]
pub struct SftpResponse {
    pub req_id : u32,
    pub packet : SftpResponsePacket,
}

#[derive(Debug)]
pub enum SftpResponsePacket {
    Version(FxpVersion),
    Status(FxpStatus),
    Handle(FxpHandle),
    Attrs(FileAttr),
    Unknown{msg_type: u8, data: Vec<u8>},
}

impl Sendable for Vec<u8> {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        let mut n : usize = 0;
        try!(w.write_u32::<BigEndian>(self.len() as u32));
        n += 4;
        try!(w.write_all(self.as_slice()));
        n += self.len();
        Ok(n)
    }
}

impl Receivable for Vec<u8> {
    fn recv<R : io::Read>(r: &mut io::Take<R>) -> Result<Vec<u8>> {
        let l = try!(r.read_u32::<BigEndian>());
        let mut s = Vec::with_capacity(l as usize);
        let mut lr = r.take(l as u64);
        let n = try!(lr.read_to_end(&mut s));
        if n != l as usize {
            io::Error::new(io::ErrorKind::Other, "short read");
        }
        Ok(s)
    }
}

#[derive(Debug)]
pub struct Extension {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

impl Sendable for Extension {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        let mut n : usize = 0;
        n += try!(self.name.write_to(w));
        n += try!(self.data.write_to(w));
        Ok(n)
    }
}

impl Receivable for Extension {
    fn recv<R : io::Read>(r: &mut io::Take<R>) -> Result<Extension> {
        let name = try!(Vec::<u8>::recv(r));
        let data = try!(Vec::<u8>::recv(r));
        Ok(Extension{name: name, data: data})
    }
}

const SSH_FILEXFER_ATTR_SIZE : u32 = 0x00000001;
const SSH_FILEXFER_ATTR_UIDGID : u32 = 0x00000002;
const SSH_FILEXFER_ATTR_PERMISSIONS : u32 = 0x00000004;
const SSH_FILEXFER_ATTR_ACMODTIME : u32 = 0x00000008;
const SSH_FILEXFER_ATTR_EXTENDED : u32 = 0x80000000;

#[derive(Debug)]
pub struct FileAttr {
    pub size : Option<u64>,
    pub uid : Option<u32>,
    pub gid : Option<u32>,
    pub perms : Option<u32>,
    pub atime : Option<u32>,
    pub mtime : Option<u32>,
    pub extensions : Vec<Extension>,
}

impl FileAttr {
    pub fn new() -> FileAttr {
        FileAttr{
            size: None,
            uid: None,
            gid: None,
            perms: None,
            atime: None,
            mtime: None,
            extensions: Vec::new()
        }
    }
}

impl Sendable for FileAttr {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        let mut flags : u32 = 0;
        if self.size.is_some() {
            flags &= SSH_FILEXFER_ATTR_SIZE;
        }
        if self.uid.is_some() && self.gid.is_some() {
            flags &= SSH_FILEXFER_ATTR_UIDGID;
        }
        if self.perms.is_some() {
            flags &= SSH_FILEXFER_ATTR_PERMISSIONS;
        }
        if self.atime.is_some() && self.mtime.is_some() {
            flags &= SSH_FILEXFER_ATTR_ACMODTIME;
        }
        if self.extensions.len() > 0 {
            flags &= SSH_FILEXFER_ATTR_EXTENDED;
        }
        let mut n : usize = 0;
        try!(w.write_u32::<BigEndian>(flags));
        n += 4;
        if let Some(size) = self.size {
            try!(w.write_u64::<BigEndian>(size));
            n += 8;
        }
        match (self.uid, self.gid) {
            (Some(uid), Some(gid)) => {
                try!(w.write_u32::<BigEndian>(uid));
                try!(w.write_u32::<BigEndian>(gid));
                n += 8;
            },
            _ => {},
        }
        if let Some(perms) = self.perms {
            try!(w.write_u32::<BigEndian>(perms));
            n += 4;
        }
        match (self.atime, self.mtime) {
            (Some(atime), Some(mtime)) => {
                try!(w.write_u32::<BigEndian>(atime));
                try!(w.write_u32::<BigEndian>(mtime));
                n += 8;
            },
            _ => {},
        }
        for extension in self.extensions.iter() {
            n += try!(extension.write_to(w));
        }
        Ok(n)
    }
}

impl Receivable for FileAttr {
    fn recv<R: io::Read>(r: &mut io::Take<R>) -> Result<FileAttr> {
        let flags = try!(r.read_u32::<BigEndian>());
        let size = if flags & SSH_FILEXFER_ATTR_SIZE != 0 {
            Some(try!(r.read_u64::<BigEndian>()))
        } else {
            None
        };
        let (uid, gid) = if flags & SSH_FILEXFER_ATTR_UIDGID != 0 {
            (Some(try!(r.read_u32::<BigEndian>())), Some(try!(r.read_u32::<BigEndian>())))
        } else {
            (None, None)
        };
        let perms = if flags & SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            Some(try!(r.read_u32::<BigEndian>()))
        } else {
            None
        };
        let (atime, mtime) = if flags & SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            (Some(try!(r.read_u32::<BigEndian>())), Some(try!(r.read_u32::<BigEndian>())))
        } else {
            (None, None)
        };
        let extensions = if flags & SSH_FILEXFER_ATTR_EXTENDED != 0 {
            let ext_count = try!(r.read_u32::<BigEndian>());
            let mut extensions = Vec::new();
            for _ in 0..ext_count {
                extensions.push(try!(Extension::recv(r)));
            }
            extensions
        } else {
            Vec::new()
        };
        Ok(FileAttr{size: size, uid: uid, gid: gid, perms: perms, atime: atime, mtime: mtime, extensions: extensions})
    }
}

#[derive(Debug)]
pub struct FxpInit {
	pub version: u32,
	pub extensions: Vec<Extension>,
}

impl Request for FxpInit {
    fn msg_type() -> u8 { SSH_FXP_INIT }
}

impl Sendable for FxpInit {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        let mut n : usize = 0;
        try!(w.write_u32::<BigEndian>(self.version));
        n += 4;
        for e in self.extensions.iter() {
            n += try!(e.write_to(w));
        }
        Ok(n)
    }
}

#[derive(Debug)]
pub struct FxpOpen {
    pub filename : Vec<u8>,
    pub pflags : u32,
    pub attrs : FileAttr,
}

impl Request for FxpOpen {
    fn msg_type() -> u8 { SSH_FXP_OPEN }
}

impl Sendable for FxpOpen {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        let mut n : usize = 0;
        n += try!(self.filename.write_to(w));
        try!(w.write_u32::<BigEndian>(self.pflags));
        n += 4;
        n += try!(self.attrs.write_to(w));
        Ok(n)
    }
}

#[derive(Debug)]
pub struct FxpFStat {
    pub handle : Vec<u8>
}

impl Request for FxpFStat {
    fn msg_type() -> u8 { SSH_FXP_FSTAT }
}

impl Sendable for FxpFStat {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        Ok(try!(self.handle.write_to(w)))
    }
}

#[derive(Debug)]
pub struct FxpStat {
    pub path : Vec<u8>
}

impl Request for FxpStat {
    fn msg_type() -> u8 { SSH_FXP_STAT }
}

impl Sendable for FxpStat {
    fn write_to<W : io::Write>(&self, w: &mut W) -> Result<usize> {
        Ok(try!(self.path.write_to(w)))
    }
}

#[derive(Debug)]
pub struct FxpVersion {
    pub version: u32,
    pub extensions: Vec<Extension>,
}

impl Response for FxpVersion {
    fn msg_type() -> u8 { SSH_FXP_VERSION }
}

impl Receivable for FxpVersion {
    fn recv<R : io::Read>(r: &mut io::Take<R>) -> Result<FxpVersion> {
            let version = try!(r.read_u32::<BigEndian>());
            let mut extensions = Vec::new();
            while r.limit() > 0 {
                extensions.push(try!(Extension::recv(r)));
            }
            Ok(FxpVersion{version: version, extensions: extensions})
    }
}

const SSH_FX_OK : u32 = 0;
const SSH_FX_EOF : u32 = 1;
const SSH_FX_NO_SUCH_FILE : u32 = 2;
const SSH_FX_PERMISSION_DENIED : u32 = 3;
const SSH_FX_FAILURE : u32 = 4;
const SSH_FX_BAD_MESSAGE : u32 = 5;
const SSH_FX_NO_CONNECTION : u32 = 6;
const SSH_FX_CONNECTION_LOST : u32 = 7;
const SSH_FX_OP_UNSUPPORTED : u32 = 8;


#[derive(Debug)]
pub enum FxpStatus {
    Ok{msg: Vec<u8>},
    EOF{msg: Vec<u8>},
    NoSuchFile{msg: Vec<u8>},
    PermissionDenied{msg: Vec<u8>},
    Failure{msg: Vec<u8>},
    BadMessage{msg: Vec<u8>},
    NoConnection{msg: Vec<u8>},
    ConnectionLost{msg: Vec<u8>},
    OpUnsupported{msg: Vec<u8>},
    UnknownCode{msg: Vec<u8>, data: Vec<u8>},
}

impl Response for FxpStatus {
    fn msg_type() -> u8 { SSH_FXP_STATUS }
}

impl Receivable for FxpStatus {
    fn recv<R: io::Read>(r: &mut io::Take<R>) -> Result<FxpStatus> {
        let code = try!(r.read_u32::<BigEndian>());
        let msg = try!(Vec::<u8>::recv(r));
        try!(Vec::<u8>::recv(r));  // Skip lang

        match code {
            SSH_FX_OK => Ok(FxpStatus::Ok{msg: msg}),
            SSH_FX_EOF => Ok(FxpStatus::EOF{msg: msg}),
            SSH_FX_NO_SUCH_FILE => Ok(FxpStatus::NoSuchFile{msg: msg}),
            SSH_FX_PERMISSION_DENIED => Ok(FxpStatus::PermissionDenied{msg: msg}),
            SSH_FX_FAILURE => Ok(FxpStatus::Failure{msg: msg}),
            SSH_FX_BAD_MESSAGE => Ok(FxpStatus::BadMessage{msg: msg}),
            SSH_FX_NO_CONNECTION => Ok(FxpStatus::NoConnection{msg: msg}),
            SSH_FX_CONNECTION_LOST => Ok(FxpStatus::ConnectionLost{msg: msg}),
            SSH_FX_OP_UNSUPPORTED => Ok(FxpStatus::OpUnsupported{msg: msg}),
            _ => {
                let mut data = Vec::new();
                try!(r.read_to_end(&mut data));
                Ok(FxpStatus::UnknownCode{msg: msg, data: data})
            },
        }
    }
}

#[derive(Debug)]
pub struct FxpHandle {
    pub handle: Vec<u8>,
}

impl Response for FxpHandle {
    fn msg_type() -> u8 { SSH_FXP_HANDLE }
}

impl Receivable for FxpHandle {
    fn recv<R: io::Read>(r: &mut io::Take<R>) -> Result<FxpHandle> {
        Ok(FxpHandle{handle: try!(Vec::<u8>::recv(r))})
    }
}

pub fn recv<R : io::Read>(r: &mut R) -> Result<SftpResponse> {
    let l = try!(r.read_u32::<BigEndian>());
    let mut lr = r.take(l as u64);
    let msg_type = {
        let mut x : [u8; 1] = [0];
        if try!(lr.read(&mut x[..])) < 1 {
            return Err(Error::UnexpectedEOF)
        }
        x[0]
    };
    // SSH_FXP_VERSION is the one response that is returned without a request id. Hardcode it to
    // zero.
    let req_id = if msg_type == SSH_FXP_VERSION {
        0
    } else {
        try!(lr.read_u32::<BigEndian>())
    };
    let response = if msg_type == SSH_FXP_VERSION {
        SftpResponsePacket::Version(try!(FxpVersion::recv(&mut lr)))
    } else if msg_type == SSH_FXP_STATUS {
        SftpResponsePacket::Status(try!(FxpStatus::recv(&mut lr)))
    } else if msg_type == SSH_FXP_HANDLE {
        SftpResponsePacket::Handle(try!(FxpHandle::recv(&mut lr)))
    } else if msg_type == SSH_FXP_ATTRS {
        SftpResponsePacket::Attrs(try!(FileAttr::recv(&mut lr)))
    } else {
        let mut data = Vec::new();
        try!(lr.read_to_end(&mut data));
        SftpResponsePacket::Unknown{msg_type: msg_type, data: data}
    };
    if lr.limit() > 0 {
        return Err(Error::UnexpectedData)
    }
    Ok(SftpResponse{req_id: req_id, packet: response})
}

