//! Only validated numeric version strings or constraints may enter diagnostics.
use semver::{Version, VersionReq};

pub(crate) fn version(s: &str) -> Option<&str> {
    let s = s.trim().trim_start_matches('v');
    let mut digit = true;
    if s.len() > 64 || s.is_empty() {
        return None;
    }
    for ch in s.bytes() {
        match ch {
            b'0'..=b'9' => digit = false,
            b'.' | b'_' | b'+' | b'-' if !digit => digit = true,
            _ => return None,
        }
    }
    (!digit).then_some(s)
}
fn parts(s: &str) -> Option<Vec<u64>> {
    let s = version(s)?;
    s.split(['+', '-'])
        .next()?
        .split(['.', '_'])
        .map(|v| v.parse().ok())
        .collect()
}
fn clause(s: &str) -> Option<(&str, &str)> {
    let op = [">=", "<=", "==", ">", "<", "=", "^", "~"]
        .into_iter()
        .find(|p| s.starts_with(p))
        .unwrap_or("");
    let n = s[op.len()..].trim_end_matches(".*").trim_end_matches(".x");
    version(n)?;
    Some((op, n))
}
pub fn valid_requirement(s: &str) -> bool {
    s == "*"
        || (!s.is_empty()
            && s.len() <= 96
            && s.split("||").all(|group| {
                let mut tokens = group.split([',', ' ']).filter(|s| !s.is_empty()).peekable();
                tokens.peek().is_some() && tokens.all(|t| clause(t).is_some())
            }))
}
pub(crate) fn matches(actual: &str, requirement: &str) -> bool {
    if requirement == "*" {
        return true;
    }
    let Some(actual) = parts(actual) else {
        return false;
    };
    requirement.split("||").any(|group| {
        group.split([',', ' ']).filter(|s| !s.is_empty()).all(|s| {
            let Some((op, n)) = clause(s) else {
                return false;
            };
            let Some(mut want) = parts(n) else {
                return false;
            };
            if op.is_empty() || op == "==" || op == "=" {
                return actual.starts_with(&want);
            }
            if op == "^" || op == "~" {
                let v = Version::new(
                    actual[0],
                    *actual.get(1).unwrap_or(&0),
                    *actual.get(2).unwrap_or(&0),
                );
                return VersionReq::parse(s).is_ok_and(|r| r.matches(&v));
            }
            let mut have = actual.clone();
            let len = have.len().max(want.len());
            want.resize(len, 0);
            have.resize(len, 0);
            match op {
                ">=" => have >= want,
                "<=" => have <= want,
                ">" => have > want,
                "<" => have < want,
                _ => false,
            }
        })
    })
}
