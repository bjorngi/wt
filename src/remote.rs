//! Parse `origin` URLs into host / owner / name.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// `git@host:owner/name.git`, `ssh://git@host[:port]/owner/name`, `https://[user@]host/owner/name[.git]`.
/// Local paths and URLs whose path isn't exactly `owner/name` give `None`.
pub fn parse(url: &str) -> Option<Remote> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        (host.split(':').next()?, path)
    } else {
        // scp-like syntax: [user@]host:path
        let (left, path) = url.split_once(':')?;
        if left.contains('/') {
            return None;
        }
        (left.rsplit('@').next()?, path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(Remote { host: host.to_ascii_lowercase(), owner: owner.to_string(), name: name.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(host: &str, owner: &str, name: &str) -> Option<Remote> {
        Some(Remote { host: host.into(), owner: owner.into(), name: name.into() })
    }

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse("git@github.com:acme/api.git"), r("github.com", "acme", "api"));
        assert_eq!(parse("git@github.com:acme/api"), r("github.com", "acme", "api"));
        assert_eq!(parse("https://github.com/acme/api.git"), r("github.com", "acme", "api"));
        assert_eq!(parse("https://github.com/acme/api/"), r("github.com", "acme", "api"));
        assert_eq!(parse("https://user@GitHub.com/acme/api"), r("github.com", "acme", "api"));
        assert_eq!(parse("ssh://git@github.example.com:2222/acme/api.git"), r("github.example.com", "acme", "api"));
    }

    #[test]
    fn rejects_local_and_odd_paths() {
        assert_eq!(parse("/tmp/remotes/api.git"), None);
        assert_eq!(parse("file:///tmp/api.git"), None);
        assert_eq!(parse("../api"), None);
        assert_eq!(parse("https://gitlab.com/group/sub/api.git"), None);
    }
}
