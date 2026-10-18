use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use crate::i18n::tr;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Spawn {
        program: String,
        source: io::Error,
    },
    Failed {
        command: String,
        status: ExitStatus,
        output: String,
    },
    Write {
        path: PathBuf,
        source: io::Error,
    },
    Terminal(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(text) => write!(formatter, "{text}"),
            Error::Spawn { program, source } => write!(
                formatter,
                "{}",
                tr!(
                    "failed to start {program}: {source}",
                    "не удалось запустить {program}: {source}"
                )
            ),
            Error::Failed {
                command,
                status,
                output,
            } => write!(formatter, "{command}: {status}\n{output}"),
            Error::Write { path, source } => write!(formatter, "{}: {source}", path.display()),
            Error::Terminal(source) => write!(
                formatter,
                "{}",
                tr!("terminal: {source}", "терминал: {source}")
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Spawn { source, .. } | Error::Write { source, .. } | Error::Terminal(source) => {
                Some(source)
            }
            Error::Invalid(_) | Error::Failed { .. } => None,
        }
    }
}
