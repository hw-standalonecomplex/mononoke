// Copyright (c) 2018-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

pub use failure::{Error, ResultExt};

use MPath;

#[derive(Debug, Fail)]
pub enum ErrorKind {
    #[fail(display = "invalid blake2 input: {}", _0)] InvalidBlake2Input(String),
    #[fail(display = "invalid path '{}': {}", _0, _1)] InvalidPath(String, String),
    #[fail(display = "invalid Mononoke path '{}': {}", _0, _1)] InvalidMPath(MPath, String),
    #[fail(display = "invalid Thrift structure '{}': {}", _0, _1)] InvalidThrift(String, String),
}

pub type Result<T> = ::std::result::Result<T, Error>;
