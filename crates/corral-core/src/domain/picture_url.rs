use crate::{Error, Result};

const MAX_LEN: usize = 2048;
const SCHEME: &str = "https://";

validated_string!(PictureUrl, check);

/// An absolute `https://` URL with a host and no credentials, in printable ASCII, as identity
/// providers send it.
fn check(value: &str) -> Result<()> {
    let authority = value
        .strip_prefix(SCHEME)
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
    let well_formed = value.len() <= MAX_LEN
        && value.bytes().all(|b| b.is_ascii_graphic())
        && authority.is_some_and(|authority| {
            !authority.is_empty() && !authority.starts_with(':') && !authority.contains('@')
        });
    if well_formed {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "picture {value:?} must be an absolute https URL"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::PictureUrl;
    use crate::Error;

    #[test]
    fn only_absolute_https_urls_with_a_host_are_pictures() {
        for good in [
            "https://idp.example.com/avatar.png",
            "https://idp.example.com",
            "https://idp.example.com:8443/a?size=64#x",
            "https://[2001:db8::1]/a.png",
            "https://cdn.example.com/a%20b.png",
        ] {
            assert!(PictureUrl::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "http://idp.example.com/avatar.png",
            "//idp.example.com/avatar.png",
            "/avatar.png",
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "https://",
            "https:///avatar.png",
            "https://:443/avatar.png",
            "https://user:pass@idp.example.com/a.png",
            "https://idp.example.com/a b.png",
            "https://idp.example.com/\u{e9}.png",
            "https://idp.example.com/\n",
            "HTTPS://idp.example.com/a.png",
            "",
            &format!("https://idp.example.com/{}", "a".repeat(2048)),
        ] {
            assert!(
                matches!(PictureUrl::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }
}
