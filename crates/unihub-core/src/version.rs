//! Unity version strings such as `6000.0.85f1`, `2022.3.10f1c1` or `6000.3.0b4`.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// `a` alpha, `b` beta, `f` final, `p` patch release, anything else sorts with `f`.
    pub kind: char,
    pub build: u32,
    /// Trailing text, e.g. `c1` for China builds.
    pub suffix: String,
}

impl UnityVersion {
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let rest = parts.next()?;

        let digits_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let patch = rest[..digits_end].parse().ok()?;
        let rest = &rest[digits_end..];

        let mut chars = rest.chars();
        let kind = chars.next().unwrap_or('f');
        let rest = chars.as_str();
        let build_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let build = rest[..build_end].parse().unwrap_or(0);
        let suffix = rest[build_end..].to_string();

        Some(Self { major, minor, patch, kind, build, suffix })
    }

    fn kind_rank(&self) -> u8 {
        match self.kind {
            'a' => 0,
            'b' => 1,
            'p' => 3,
            _ => 2,
        }
    }
}

impl Ord for UnityVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch, self.kind_rank(), self.build, &self.suffix).cmp(&(
            other.major,
            other.minor,
            other.patch,
            other.kind_rank(),
            other.build,
            &other.suffix,
        ))
    }
}

impl PartialOrd for UnityVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Orders two version strings. Unparseable strings sort before parseable ones.
pub fn compare(a: &str, b: &str) -> Ordering {
    match (UnityVersion::parse(a), UnityVersion::parse(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        let v = UnityVersion::parse("6000.0.85f1").unwrap();
        assert_eq!((v.major, v.minor, v.patch, v.kind, v.build), (6000, 0, 85, 'f', 1));
        let v = UnityVersion::parse("2020.3.49f1c1").unwrap();
        assert_eq!(v.suffix, "c1");
        assert!(UnityVersion::parse("garbage").is_none());
    }

    #[test]
    fn orders_versions() {
        assert_eq!(compare("6000.3.0b4", "6000.3.0f1"), Ordering::Less);
        assert_eq!(compare("6000.0.9f1", "6000.0.85f1"), Ordering::Less);
        assert_eq!(compare("2022.3.10f1", "6000.0.1f1"), Ordering::Less);
        assert_eq!(compare("6000.0.85f2", "6000.0.85f1"), Ordering::Greater);
    }
}
