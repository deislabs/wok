use std::convert::TryFrom;

// currently, the library only accepts modules tagged in the following structure:
// <registry>/<repository>:<tag>
// for example: webassembly.azurecr.io/hello:v1
#[derive(Copy, Clone)]
pub struct ImageReference<'a> {
    pub(crate) whole: &'a str,
    pub(crate) registry: &'a str,
    pub(crate) repo: &'a str,
    pub(crate) tag: &'a str,
}

impl<'a> TryFrom<&'a String> for ImageReference<'a> {
    type Error = ();
    fn try_from(string: &'a String) -> Result<Self, Self::Error> {
        let mut registry_parts = string.split('/');
        let registry = registry_parts.next().ok_or(())?;
        let mut repo_parts = registry_parts.next().ok_or(())?.split(':');
        let repo = repo_parts.next().ok_or(())?;
        let tag = repo_parts.next().ok_or(())?;
        Ok(ImageReference {
            whole: string,
            registry,
            repo,
            tag,
        })
    }
}
