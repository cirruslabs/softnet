use anyhow::{anyhow, Context, Error};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExposedPort {
    pub external_port: u16,
    pub internal_port: u16,
}

impl FromStr for ExposedPort {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splits: Vec<&str> = s.split(':').collect();

        match splits.len() {
            2 => Ok(ExposedPort {
                external_port: splits[0]
                    .parse()
                    .context(format!("invalid external port {:?}", splits[0]))?,
                internal_port: splits[1]
                    .parse()
                    .context(format!("invalid internal port {:?}", splits[1]))?,
            }),
            _ => Err(anyhow!(
                "invalid exposed port specification {:?}, the format should be EXTERNAL:INTERNAL",
                s
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::proxy::exposed_port::ExposedPort;

    #[test]
    fn exposed_port() {
        assert_eq!(
            ExposedPort {
                external_port: 2222,
                internal_port: 22
            },
            "2222:22".parse().unwrap()
        );
    }
}
