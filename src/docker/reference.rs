use std::convert::{Into, TryFrom};

// currently, the library only accepts modules tagged in the following structure:
// <registry>/<repository>:<tag>
// for example: webassembly.azurecr.io/hello:v1
#[derive(Clone)]
pub struct Reference {
    whole: String,
    slash: usize,
    colon: usize,
}

impl Reference {
    pub fn whole(&self) -> &str {
        &self.whole
    }

    pub fn registry(&self) -> &str {
        &self.whole[..self.slash]
    }

    pub fn repository(&self) -> &str {
        &self.whole[self.slash + 1..self.colon]
    }

    pub fn tag(&self) -> &str {
        &self.whole[self.colon + 1..]
    }
}

impl TryFrom<String> for Reference {
    type Error = ();
    fn try_from(string: String) -> Result<Self, Self::Error> {
        let slash = string.find('/').ok_or(())?;
        let colon = string[slash + 1..].find(':').ok_or(())?;
        Ok(Reference {
            whole: string,
            slash,
            colon: slash + 1 + colon,
        })
    }
}

impl Into<String> for Reference {
    fn into(self) -> String {
        self.whole
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correctly_parses_string() {
        let reference = Reference::try_from("webassembly.azurecr.io/hello:v1".to_owned())
            .expect("Could not parse reference");

        assert_eq!(reference.registry(), "webassembly.azurecr.io");
        assert_eq!(reference.repository(), "hello");
        assert_eq!(reference.tag(), "v1");
    }
}
