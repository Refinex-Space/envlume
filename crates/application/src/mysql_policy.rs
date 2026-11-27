use crate::{ErrorCode, input::TableName};
use envlume_domain::{MySqlRoute, ResourceBinding, TableScope, TlsMode};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
pub fn public_ca(input: &[u8]) -> Result<String, ErrorCode> {
    if input.len() > 16384 {
        return Err(ErrorCode::InvalidInput);
    }
    let certs = pem::parse_many(input).map_err(|_| ErrorCode::InvalidInput)?;
    let [cert] = certs.as_slice() else {
        return Err(ErrorCode::InvalidInput);
    };
    if cert.tag() != "CERTIFICATE" {
        return Err(ErrorCode::InvalidInput);
    }
    let der = rustls_pki_types::CertificateDer::from(cert.contents());
    webpki::anchor_from_trusted_cert(&der).map_err(|_| ErrorCode::InvalidInput)?;
    Ok(pem::encode(cert))
}
pub fn ca_summary(route: &MySqlRoute) -> String {
    route.ca_pem.as_ref().map_or_else(
        || "built-in roots".into(),
        |ca| format!("{:x}", Sha256::digest(ca.as_bytes())),
    )
}
fn allowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.octets()[0] != 0
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_link_local()
                && ip.octets()[0] < 240
        }
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unicast_link_local()
                && (ip.is_loopback() || ip.to_ipv4().is_none())
        }
    }
}
pub fn validate(
    resource: &ResourceBinding,
    route: &MySqlRoute,
    tables: &TableScope,
) -> Result<(), ErrorCode> {
    if route.ips.is_empty()
        || route.ips.len() > 16
        || route.ips.iter().any(|ip| !allowed_ip(*ip))
        || resource
            .host
            .parse::<IpAddr>()
            .is_ok_and(|ip| !route.ips.contains(&ip))
        || (resource.tls == TlsMode::Disabled
            && (route.ca_pem.is_some() || route.ips.iter().any(|ip| !ip.is_loopback())))
    {
        return Err(ErrorCode::TargetNotAllowed);
    }
    if let Some(ca) = &route.ca_pem
        && public_ca(ca.as_bytes())? != *ca
    {
        return Err(ErrorCode::InvalidInput);
    }
    if tables.len() > 32 || tables.values().map(|v| v.len()).sum::<usize>() > 512 {
        return Err(ErrorCode::InvalidInput);
    }
    for (t, cols) in tables {
        if cols.is_empty()
            || TableName::try_from(t.clone()).is_err()
            || cols.iter().any(|s| TableName::try_from(s.clone()).is_err())
        {
            return Err(ErrorCode::InvalidInput);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_ipv6_loopback_is_valid_but_ipv4_mapped_link_local_is_not() {
        assert!(allowed_ip("::1".parse().unwrap()));
        for ip in [
            "::",
            "::ffff:169.254.169.254",
            "169.254.169.254",
            "ff02::1",
            "0.0.0.0",
            "255.255.255.255",
            "fe80::1",
        ] {
            assert!(!allowed_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
