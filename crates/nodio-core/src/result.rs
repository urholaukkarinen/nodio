use std::fmt::{Display, Formatter};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub enum Error {
    NoSuchDevice,
    CouldNotConnect(String),
    Other(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoSuchDevice => write!(f, "No such device"),
            Error::CouldNotConnect(reason) => write!(f, "Could not connect: {}", reason),
            Error::Other(reason) => write!(f, "{}", reason),
        }
    }
}
