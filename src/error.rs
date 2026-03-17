use std::env::VarError;
use std::fmt;
use std::io::Error as IoError;

use git2::Error as GitError;
use reqwest::Error as ReqwestError;

#[derive(Debug)]
pub enum Error {
    Git(GitError),
    Var(VarError),
    Io(IoError),
    GitHub(ReqwestError),
}

impl From<GitError> for Error {
    fn from(err: GitError) -> Error {
        Error::Git(err)
    }
}

impl From<VarError> for Error {
    fn from(err: VarError) -> Error {
        Error::Var(err)
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Error {
        Error::Io(err)
    }
}

impl From<ReqwestError> for Error {
    fn from(err: ReqwestError) -> Error {
        Error::GitHub(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Git(e) => e.fmt(f),
            Error::Var(e) => e.fmt(f),
            Error::Io(e) => e.fmt(f),
            Error::GitHub(e) => e.fmt(f),
        }
    }
}
