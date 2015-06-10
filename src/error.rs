extern crate byteorder;

use std::io;
use self::byteorder::Error as ByteError;

use packets;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    UnexpectedData,
    UnexpectedEOF,
    PreviousError,
    NoMatchingRequest(u32),
    MismatchedVersion(u32),
    UnexpectedResponse(packets::SftpResponsePacket),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}

impl From<ByteError> for Error {
    fn from(err: ByteError) -> Error {
            match err {
                ByteError::Io(ioerr) => Error::Io(ioerr),
                ByteError::UnexpectedEOF => Error::UnexpectedEOF,
            }
    }
}

pub type Result<T> = ::std::result::Result<T, Error>;

