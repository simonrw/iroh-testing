use color_eyre::eyre;
use std::str::FromStr;

pub(crate) mod echo;
pub(crate) mod reverse;

#[derive(Clone, Copy)]
pub(crate) enum Protocols {
    Echo,
    Reverse,
}

impl FromStr for Protocols {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "echo" => Ok(Self::Echo),
            "reverse" => Ok(Self::Reverse),
            _ => Err(eyre::eyre!("Unhandled protocol: {s}")),
        }
    }
}

impl Protocols {
    pub(crate) fn alpn(&self) -> &[u8] {
        match self {
            Self::Echo => echo::ALPN,
            Self::Reverse => reverse::ALPN,
        }
    }
}
