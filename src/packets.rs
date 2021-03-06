extern crate byteorder;

use self::byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};

use std::fmt;
use std::io;
use std::io::Read;
use std::error::Error as StdError;
use error::{Error, Result};

// Init
const SSH_FXP_INIT : u8 = 1;
const SSH_FXP_VERSION : u8 = 2;

// Requests
const SSH_FXP_OPEN : u8 = 3;
const SSH_FXP_CLOSE : u8 = 4;
const SSH_FXP_READ : u8 = 5;
const SSH_FXP_WRITE : u8 = 6;
const SSH_FXP_LSTAT : u8 = 7;
const SSH_FXP_FSTAT : u8 = 8;
const SSH_FXP_SETSTAT : u8 = 9;
const SSH_FXP_FSETSTAT : u8 = 10;
const SSH_FXP_OPENDIR : u8 = 11;
const SSH_FXP_READDIR : u8 = 12;
const SSH_FXP_REMOVE : u8 = 13;
const SSH_FXP_MKDIR : u8 = 14;
const SSH_FXP_RMDIR : u8 = 15;
const SSH_FXP_REALPATH : u8 = 16;
const SSH_FXP_STAT : u8 = 17;
const SSH_FXP_RENAME : u8 = 18;
const SSH_FXP_READLINK : u8 = 19;
// SSH_FXP_SYMLINK is not implemented because openssh sftp server reversed the order of the
// arguments, making it incompatible with the rfc and other implementations.
//const SSH_FXP_SYMLINK : u8 = 20;

// Responses
const SSH_FXP_STATUS : u8 = 101;
const SSH_FXP_HANDLE : u8 = 102;
const SSH_FXP_DATA : u8 = 103;
const SSH_FXP_NAME : u8 = 104;
const SSH_FXP_ATTRS : u8 = 105;
//const SSH_FXP_EXTENDED : u8 = 200;
//const SSH_FXP_EXTENDED_REPLY : u8 = 201;

pub trait Request : fmt::Debug + Sendable {
    fn msg_type() -> u8;
}

pub trait Sendable {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()>;

    fn size(&self) -> u32;
}

pub trait Response : fmt::Debug + Receivable {
    fn msg_type() -> u8;
}

pub trait Receivable {
    fn recv<R: io::Read>(r: &mut R) -> Result<Self>;
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
    Data(FxpData),
    Name(FxpName),
    Attrs(FileAttr),
    Unknown{msg_type: u8, data: Vec<u8>},
}

impl Sendable for u8 {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(w.write_all(&[*self])))
    }

    fn size(&self) -> u32 {
        1
    }
}

impl Receivable for u8 {
    fn recv<R: io::Read>(r: &mut R) -> Result<u8> {
        let mut buf = [0;1];
        if try!(r.read(&mut buf)) < 1 {
            return Err(Error::UnexpectedEOF)
        }
        Ok(buf[0])
    }
}

impl Sendable for u32 {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(w.write_u32::<BigEndian>(*self)))
    }

    fn size(&self) -> u32 {
        4
    }
}

impl Receivable for u32 {
    fn recv<R: io::Read>(r: &mut R) -> Result<u32> {
        Ok(try!(r.read_u32::<BigEndian>()))
    }
}

impl Sendable for u64 {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(w.write_u64::<BigEndian>(*self)))
    }

    fn size(&self) -> u32 {
        8
    }
}

impl Receivable for u64 {
    fn recv<R: io::Read>(r: &mut R) -> Result<u64> {
        Ok(try!(r.read_u64::<BigEndian>()))
    }
}

impl Sendable for Vec<u8> {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(w.write_u32::<BigEndian>(self.len() as u32));
        Ok(try!(w.write_all(self.as_slice())))
    }

    fn size(&self) -> u32 {
        4 + self.len() as u32
    }
}

impl Receivable for Vec<u8> {
    fn recv<R: io::Read>(r: &mut R) -> Result<Vec<u8>> {
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

impl<T> Sendable for Option<T> where T : Sendable {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        match *self {
            Some(ref x) => Ok(try!(x.write_to(w))),
            None => Ok(()),
        }
    }

    fn size(&self) -> u32 {
        match *self {
            Some(ref x) => x.size(),
            None => 0,
        }
    }
}

impl<T> Receivable for Option<T> where T : Receivable {
    fn recv<R: io::Read>(r: &mut R) -> Result<Option<T>> {
        Ok(Some(try!(T::recv(r))))
    }
}

#[derive(Debug)]
pub struct Extension {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

impl Sendable for Extension {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.name.write_to(w));
        Ok(try!(self.data.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.name.size() + self.data.size()
    }
}

impl Receivable for Extension {
    fn recv<R: io::Read>(r: &mut R) -> Result<Extension> {
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
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        let mut flags : u32 = 0;
        if self.size.is_some() {
            flags |= SSH_FILEXFER_ATTR_SIZE;
        }
        if self.uid.is_some() && self.gid.is_some() {
            flags |= SSH_FILEXFER_ATTR_UIDGID;
        }
        if self.perms.is_some() {
            flags |= SSH_FILEXFER_ATTR_PERMISSIONS;
        }
        if self.atime.is_some() && self.mtime.is_some() {
            flags |= SSH_FILEXFER_ATTR_ACMODTIME;
        }
        if self.extensions.len() > 0 {
            flags |= SSH_FILEXFER_ATTR_EXTENDED;
        }
        try!(flags.write_to(w));
        try!(self.size.write_to(w));
        try!(self.uid.write_to(w));
        try!(self.gid.write_to(w));
        try!(self.atime.write_to(w));
        try!(self.mtime.write_to(w));
        for extension in self.extensions.iter() {
            try!(extension.write_to(w));
        }
        Ok(())
    }

    fn size(&self) -> u32 {
        return 4 + self.size.size() + self.uid.size() + self.gid.size() + self.atime.size() +
            self.mtime.size() + self.extensions.iter().fold(0, |acc, e| acc + e.size());
    }
}

impl Receivable for FileAttr {
    fn recv<R: io::Read>(r: &mut R) -> Result<FileAttr> {
        let flags = try!(r.read_u32::<BigEndian>());
        let size = if flags & SSH_FILEXFER_ATTR_SIZE != 0 {
            try!(Option::<u64>::recv(r))
        } else {
            None
        };
        let (uid, gid) = if flags & SSH_FILEXFER_ATTR_UIDGID != 0 {
            (try!(Option::<u32>::recv(r)), try!(Option::<u32>::recv(r)))
        } else {
            (None, None)
        };
        let perms = if flags & SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            try!(Option::<u32>::recv(r))
        } else {
            None
        };
        let (atime, mtime) = if flags & SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            (try!(Option::<u32>::recv(r)), try!(Option::<u32>::recv(r)))
        } else {
            (None, None)
        };
        let extensions = if flags & SSH_FILEXFER_ATTR_EXTENDED != 0 {
            let ext_count = try!(u32::recv(r));
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
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.version.write_to(w));
        for e in self.extensions.iter() {
            try!(e.write_to(w));
        }
        Ok(())
    }

    fn size(&self) -> u32 {
        self.version.size() + self.extensions.iter().fold(0, |acc, e| acc + e.size())
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
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.filename.write_to(w));
        try!(self.pflags.write_to(w));
        Ok(try!(self.attrs.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.filename.size() + self.pflags.size() + self.attrs.size()
    }
}

#[derive(Debug)]
pub struct FxpClose {
    pub handle: Vec<u8>,
}

impl Request for FxpClose {
    fn msg_type() -> u8 { SSH_FXP_CLOSE }
}

impl Sendable for FxpClose {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.handle.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size()
    }
}

#[derive(Debug)]
pub struct FxpRead {
    pub handle: Vec<u8>,
    pub offset: u64,
    pub len: u32,
}

impl Request for FxpRead {
    fn msg_type() -> u8 { SSH_FXP_READ }
}

impl Sendable for FxpRead {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.handle.write_to(w));
        try!(self.offset.write_to(w));
        Ok(try!(self.len.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size() + self.offset.size() + self.len.size()
    }
}

#[derive(Debug)]
pub struct FxpWrite {
    pub handle: Vec<u8>,
    pub offset: u64,
    pub data: Vec<u8>,
}

impl Request for FxpWrite {
    fn msg_type() -> u8 { SSH_FXP_WRITE }
}

impl Sendable for FxpWrite {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.handle.write_to(w));
        try!(self.offset.write_to(w));
        Ok(try!(self.data.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size() + self.offset.size() + self.data.size()
    }
}

#[derive(Debug)]
pub struct FxpLStat {
    pub path : Vec<u8>
}

impl Request for FxpLStat {
    fn msg_type() -> u8 { SSH_FXP_LSTAT }
}

impl Sendable for FxpLStat {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
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
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.handle.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size()
    }
}

#[derive(Debug)]
pub struct FxpSetStat {
    pub path : Vec<u8>,
    pub attrs : FileAttr,
}

impl Request for FxpSetStat {
    fn msg_type() -> u8 { SSH_FXP_SETSTAT }
}

impl Sendable for FxpSetStat {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.path.write_to(w));
        Ok(try!(self.attrs.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size() + self.attrs.size()
    }
}

#[derive(Debug)]
pub struct FxpFSetStat {
    pub handle : Vec<u8>,
    pub attrs : FileAttr,
}

impl Request for FxpFSetStat {
    fn msg_type() -> u8 { SSH_FXP_FSETSTAT }
}

impl Sendable for FxpFSetStat {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.handle.write_to(w));
        Ok(try!(self.attrs.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size() + self.attrs.size()
    }
}

#[derive(Debug)]
pub struct FxpOpenDir {
    pub path : Vec<u8>,
}

impl Request for FxpOpenDir {
    fn msg_type() -> u8 { SSH_FXP_OPENDIR }
}

impl Sendable for FxpOpenDir {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
    }
}

#[derive(Debug)]
pub struct FxpReadDir {
    pub handle : Vec<u8>,
}

impl Request for FxpReadDir {
    fn msg_type() -> u8 { SSH_FXP_READDIR }
}

impl Sendable for FxpReadDir {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.handle.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.handle.size()
    }
}

#[derive(Debug)]
pub struct FxpRemove {
    pub filename : Vec<u8>
}

impl Request for FxpRemove {
    fn msg_type() -> u8 { SSH_FXP_REMOVE }
}

impl Sendable for FxpRemove {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.filename.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.filename.size()
    }
}

#[derive(Debug)]
pub struct FxpMkDir {
    pub path : Vec<u8>,
    pub attrs : FileAttr,
}

impl Request for FxpMkDir {
    fn msg_type() -> u8 { SSH_FXP_MKDIR }
}

impl Sendable for FxpMkDir {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.path.write_to(w));
        Ok(try!(self.attrs.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size() + self.attrs.size()
    }
}

#[derive(Debug)]
pub struct FxpRmDir {
    pub path : Vec<u8>
}

impl Request for FxpRmDir {
    fn msg_type() -> u8 { SSH_FXP_RMDIR }
}

impl Sendable for FxpRmDir {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
    }
}

#[derive(Debug)]
pub struct FxpRealPath {
    pub path : Vec<u8>
}

impl Request for FxpRealPath {
    fn msg_type() -> u8 { SSH_FXP_REALPATH }
}

impl Sendable for FxpRealPath {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
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
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
    }
}

#[derive(Debug)]
pub struct FxpRename {
    pub oldpath : Vec<u8>,
    pub newpath : Vec<u8>,
}

impl Request for FxpRename {
    fn msg_type() -> u8 { SSH_FXP_RENAME }
}

impl Sendable for FxpRename {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        try!(self.oldpath.write_to(w));
        Ok(try!(self.newpath.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.oldpath.size() + self.newpath.size()
    }
}

#[derive(Debug)]
pub struct FxpReadLink {
    pub path : Vec<u8>
}

impl Request for FxpReadLink {
    fn msg_type() -> u8 { SSH_FXP_READLINK }
}

impl Sendable for FxpReadLink {
    fn write_to<W: io::Write>(&self, w: &mut W) -> Result<()> {
        Ok(try!(self.path.write_to(w)))
    }

    fn size(&self) -> u32 {
        self.path.size()
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
    fn recv<R: io::Read>(r: &mut R) -> Result<FxpVersion> {
            let version = try!(u32::recv(r));
            let mut bytes = Vec::new();
            let limit = try!(r.read_to_end(&mut bytes));
            let mut extensions = Vec::new();
            let mut er = io::Cursor::new(bytes);
            while er.position() < limit as u64 {
                extensions.push(try!(Extension::recv(&mut er)));
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
pub enum FxpStatusCode {
    Ok,
    EOF,
    NoSuchFile,
    PermissionDenied,
    Failure,
    BadMessage,
    NoConnection,
    ConnectionLost,
    OpUnsupported,
    UnknownCode(Vec<u8>),
}

#[derive(Debug)]
pub struct FxpStatus {
    pub code: FxpStatusCode,
    pub msg: String,
}

impl Response for FxpStatus {
    fn msg_type() -> u8 { SSH_FXP_STATUS }
}

impl Receivable for FxpStatus {
    fn recv<R: io::Read>(r: &mut R) -> Result<FxpStatus> {
        let icode = try!(u32::recv(r));
        let msg = try!(Vec::<u8>::recv(r));
        try!(Vec::<u8>::recv(r));  // Skip lang
        let code = match icode {
            SSH_FX_OK => FxpStatusCode::Ok,
            SSH_FX_EOF => FxpStatusCode::EOF,
            SSH_FX_NO_SUCH_FILE => FxpStatusCode::NoSuchFile,
            SSH_FX_PERMISSION_DENIED => FxpStatusCode::PermissionDenied,
            SSH_FX_FAILURE => FxpStatusCode::Failure,
            SSH_FX_BAD_MESSAGE => FxpStatusCode::BadMessage,
            SSH_FX_NO_CONNECTION => FxpStatusCode::NoConnection,
            SSH_FX_CONNECTION_LOST => FxpStatusCode::ConnectionLost,
            SSH_FX_OP_UNSUPPORTED => FxpStatusCode::OpUnsupported,
            _ => {
                let mut data = Vec::new();
                try!(r.read_to_end(&mut data));
                FxpStatusCode::UnknownCode(data)
            },
        };
        Ok(FxpStatus{code: code, msg: try!(String::from_utf8(msg))})
    }
}

impl StdError for FxpStatus {
    fn description(&self) -> &str {
        return self.msg.as_str();
    }

    fn cause(&self) -> Option<&StdError> { None }
}

impl fmt::Display for FxpStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.msg)
    }
}

impl From<FxpStatus> for io::Error {
    fn from(err: FxpStatus) -> io::Error {
        let ek = match err.code {
            FxpStatusCode::NoSuchFile => io::ErrorKind::NotFound,
            FxpStatusCode::PermissionDenied => io::ErrorKind::PermissionDenied,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(ek, err)
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
    fn recv<R: io::Read>(r: &mut R) -> Result<FxpHandle> {
        Ok(FxpHandle{handle: try!(Vec::<u8>::recv(r))})
    }
}

#[derive(Debug)]
pub struct FxpData {
    pub data: Vec<u8>,
}

impl Response for FxpData {
    fn msg_type() -> u8 { SSH_FXP_DATA }
}

impl Receivable for FxpData {
    fn recv<R: io::Read>(r: &mut R) -> Result<FxpData> {
        Ok(FxpData{data: try!(Vec::<u8>::recv(r))})
    }
}

#[derive(Debug)]
pub struct Name {
    pub filename: Vec<u8>,
    pub longname: Vec<u8>,
    pub attrs: FileAttr,
}

impl Receivable for Name {
    fn recv<R: io::Read>(r: &mut R) -> Result<Name> {
        let filename = try!(Vec::<u8>::recv(r));
        let longname = try!(Vec::<u8>::recv(r));
        let attrs = try!(FileAttr::recv(r));
        Ok(Name{filename: filename, longname: longname, attrs: attrs})
    }
}

#[derive(Debug)]
pub struct FxpName {
    pub names: Vec<Name>,
}

impl Response for FxpName {
    fn msg_type() -> u8 { SSH_FXP_NAME }
}

impl Receivable for FxpName {
    fn recv<R: io::Read>(r: &mut R) -> Result<FxpName> {
        let count = try!(u32::recv(r));
        let mut names = Vec::new();
        for _ in 0..count {
            names.push(try!(Name::recv(r)));
        }
        Ok(FxpName{names: names})
    }
}

pub fn recv<R: io::Read>(r: &mut R) -> Result<SftpResponse> {
    let l = try!(u32::recv(r));
    let mut lr = r.take(l as u64);
    let msg_type = try!(u8::recv(&mut lr));
    // SSH_FXP_VERSION is the one response that is returned without a request id. Hardcode it to
    // zero.
    let req_id = if msg_type == SSH_FXP_VERSION {
        0
    } else {
        try!(u32::recv(&mut lr))
    };
    let response = if msg_type == SSH_FXP_VERSION {
        SftpResponsePacket::Version(try!(FxpVersion::recv(&mut lr)))
    } else if msg_type == SSH_FXP_STATUS {
        SftpResponsePacket::Status(try!(FxpStatus::recv(&mut lr)))
    } else if msg_type == SSH_FXP_HANDLE {
        SftpResponsePacket::Handle(try!(FxpHandle::recv(&mut lr)))
    } else if msg_type == SSH_FXP_DATA {
        SftpResponsePacket::Data(try!(FxpData::recv(&mut lr)))
    } else if msg_type == SSH_FXP_NAME {
        SftpResponsePacket::Name(try!(FxpName::recv(&mut lr)))
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

