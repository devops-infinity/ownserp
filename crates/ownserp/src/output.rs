use std::fmt;
use std::io::{self, Write};

#[derive(Debug)]
pub(crate) enum PrintError {
    BrokenPipe,
    Io(io::Error),
}

impl PrintError {
    pub(crate) const fn is_broken_pipe(&self) -> bool {
        matches!(self, Self::BrokenPipe)
    }
}

impl fmt::Display for PrintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BrokenPipe => formatter.write_str("the reader closed the output"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for PrintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BrokenPipe => None,
            Self::Io(error) => Some(error),
        }
    }
}

pub(crate) fn emit(text: &str) -> Result<(), PrintError> {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{text}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Err(PrintError::BrokenPipe),
        Err(error) => Err(PrintError::Io(error)),
    }
}
