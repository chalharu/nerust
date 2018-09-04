// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use gl;
use gl::types::*;
use std::fmt;
use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Fail)]
pub enum ErrorKind {
    #[fail(display = "An enumeration parameter is not a legal enumeration for that function.")]
    InvalidEnum,
    #[fail(display = "A value parameter is not a legal value for that function.")]
    InvalidValue,
    #[fail(
        display = "The set of state for a command is not legal for the parameters given to that command."
    )]
    InvalidOperation,
    #[fail(display = "Stack pushing operation cannot be done")]
    StackOverflow,
    #[fail(display = "Stack popping operation cannot be done")]
    StackUnderflow,
    #[fail(
        display = "Performing an operation that can allocate memory, and the memory cannot be allocated."
    )]
    OutOfMemory,
    #[fail(
        display = "Doing anything that would attempt to read from or write/render to a framebuffer that is not complete."
    )]
    InvalidFramebufferOperation,
    #[fail(display = "OpenGL context has been lost")]
    ContextLost,
    // #[fail(display = "Table too large")]
    // TableTooLarge,
    #[fail(display = "unexpected error: 0x{:04X}", _0)]
    Unexpected(GLuint),
}

impl From<GLuint> for ErrorKind {
    fn from(error_code: GLuint) -> ErrorKind {
        match error_code {
            gl::INVALID_ENUM => ErrorKind::InvalidEnum,
            gl::INVALID_VALUE => ErrorKind::InvalidValue,
            gl::INVALID_OPERATION => ErrorKind::InvalidOperation,
            gl::STACK_OVERFLOW => ErrorKind::StackOverflow,
            gl::STACK_UNDERFLOW => ErrorKind::StackUnderflow,
            gl::OUT_OF_MEMORY => ErrorKind::OutOfMemory,
            gl::INVALID_FRAMEBUFFER_OPERATION => ErrorKind::InvalidFramebufferOperation,
            gl::CONTEXT_LOST => ErrorKind::ContextLost,
            // gl::TABLE_TOO_LARGE => GlError::TableTooLarge,
            _ => ErrorKind::Unexpected(error_code),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Fail)]
pub struct Error(Vec<ErrorKind>);

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0.len() > 0 {
            try!(write!(f, "{}", self.0[0]));
            for e in self.0.iter().skip(1) {
                try!(writeln!(f));
                try!(write!(f, "{}", e));
            }
        }
        Ok(())
    }
}

impl Error {
    pub fn new(inner: Vec<ErrorKind>) -> Error {
        Error(inner)
    }
}

impl From<Vec<ErrorKind>> for Error {
    fn from(error: Vec<ErrorKind>) -> Error {
        Error::new(error)
    }
}
