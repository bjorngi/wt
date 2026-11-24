use std::fmt;

/// An error meant for the user: a short message plus an optional hint on what to do.
#[derive(Debug)]
pub struct UserError {
    pub msg: String,
    pub hint: Option<String>,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for UserError {}

pub fn user(msg: impl Into<String>) -> anyhow::Error {
    UserError { msg: msg.into(), hint: None }.into()
}

pub fn hinted(msg: impl Into<String>, hint: impl Into<String>) -> anyhow::Error {
    UserError { msg: msg.into(), hint: Some(hint.into()) }.into()
}
