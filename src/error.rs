extern crate byteorder;

use std::error;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use self::byteorder::Error as ByteError;
use std::sync::Arc;
use std::string::FromUtf8Error;

use packets;

pub type Result<T> = ::std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    ReceiverDisconnected(Arc<Box<Error>>),
    Io(io::Error),
    UnexpectedData,
    UnexpectedEOF,
    Utf8(FromUtf8Error),
    NoMatchingRequest(u32),
    MismatchedVersion(u32),
    UnexpectedResponse(Box<packets::SftpResponsePacket>),
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::ReceiverDisconnected(_) => "Receiver disconnected.",
            Error::Io(ref e) => e.description(),
            Error::UnexpectedData => "Unexpected data in message.",
            Error::UnexpectedEOF => "Unexpected EOF.",
            Error::Utf8(ref e) => e.description(),
            Error::NoMatchingRequest(_) => "Response received with an unexpected request-id.",
            Error::MismatchedVersion(_) => "Server responded with an incorrect version",
            Error::UnexpectedResponse(_) => "Unexpected response",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            Error::ReceiverDisconnected(ref e) => Some(&***e),
            Error::Io(ref err) => err.cause(),
            Error::Utf8(ref err) => err.cause(),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::ReceiverDisconnected(ref reason) => write!(f, "Receiver disconnected: {}", ***reason),
            Error::Io(ref err) => err.fmt(f),
            Error::UnexpectedData => write!(f, "Unexpected data in message."),
            Error::UnexpectedEOF => write!(f, "Unexpected EOF."),
            Error::Utf8(ref err) => err.fmt(f),
            Error::NoMatchingRequest(ref req_id) => write!(f, "Response received with an unexpected request-id: {}", *req_id),
            Error::MismatchedVersion(ref ver) => write!(f, "Server responded with version {}. Only version 3 is supported.", *ver),
            Error::UnexpectedResponse(_) => write!(f, "Unexpected response"),
        }
    }
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

impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Error {
        Error::Utf8(err)
    }
}
