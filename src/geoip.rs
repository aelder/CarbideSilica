use std::fs;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use maxminddb::{Reader, geoip2, path};
use time::{Month, OffsetDateTime};

const CITY_FILE: &str = "dbip-city-lite.mmdb";
const ASN_FILE: &str = "dbip-asn-lite.mmdb";
const RELEASE_FILE: &str = "dbip-release";

#[derive(Debug, Clone, Default)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub city: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub asn: Option<u32>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoIpStatus {
    Missing,
    Current(String),
    UpdateAvailable(String),
    Invalid(String),
}

pub struct GeoIpDatabase {
    city: Reader<Vec<u8>>,
    asn: Reader<Vec<u8>>,
}

impl GeoIpDatabase {
    pub fn open(directory: &Path) -> Result<Self> {
        Ok(Self {
            city: Reader::open_readfile(directory.join(CITY_FILE))?,
            asn: Reader::open_readfile(directory.join(ASN_FILE))?,
        })
    }

    pub fn lookup(&self, ip: IpAddr) -> Option<GeoLocation> {
        if !is_public_ip(ip) {
            return None;
        }
        let city_lookup = self.city.lookup(ip).ok()?;
        let latitude = city_lookup
            .decode_path::<f64>(&path!["location", "latitude"])
            .ok()
            .flatten()?;
        let longitude = city_lookup
            .decode_path::<f64>(&path!["location", "longitude"])
            .ok()
            .flatten()?;
        let city = city_lookup.decode::<geoip2::City>().ok().flatten();
        let asn = self
            .asn
            .lookup(ip)
            .ok()
            .and_then(|lookup| lookup.decode::<geoip2::Asn>().ok().flatten());
        Some(GeoLocation {
            latitude,
            longitude,
            city: city
                .as_ref()
                .and_then(|record| record.city.names.english.map(str::to_string)),
            country: city
                .as_ref()
                .and_then(|record| record.country.names.english.map(str::to_string)),
            country_code: city
                .as_ref()
                .and_then(|record| record.country.iso_code.map(str::to_string)),
            asn: asn
                .as_ref()
                .and_then(|record| record.autonomous_system_number),
            organization: asn
                .and_then(|record| record.autonomous_system_organization.map(str::to_string)),
        })
    }
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    !match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
        }
    }
}

pub fn database_directory(cache_path: &Path) -> PathBuf {
    cache_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("geoip")
}

pub fn status(directory: &Path) -> GeoIpStatus {
    if !directory.join(CITY_FILE).is_file() || !directory.join(ASN_FILE).is_file() {
        return GeoIpStatus::Missing;
    }
    let release = match fs::read_to_string(directory.join(RELEASE_FILE)) {
        Ok(value) => value.trim().to_string(),
        Err(error) => return GeoIpStatus::Invalid(format!("release marker: {error}")),
    };
    if release == current_release() {
        GeoIpStatus::Current(release)
    } else {
        GeoIpStatus::UpdateAvailable(release)
    }
}

pub async fn download(directory: PathBuf) -> Result<String> {
    fs::create_dir_all(&directory)?;
    let release = current_release();
    let city_url = format!("https://download.db-ip.com/free/dbip-city-lite-{release}.mmdb.gz");
    let asn_url = format!("https://download.db-ip.com/free/dbip-asn-lite-{release}.mmdb.gz");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let city = fetch(&client, &city_url)
        .await
        .context("download City Lite")?;
    let asn = fetch(&client, &asn_url)
        .await
        .context("download ASN Lite")?;
    let target = directory.clone();
    let release_copy = release.clone();
    tokio::task::spawn_blocking(move || install(&target, &release_copy, &city, &asn)).await??;
    Ok(release)
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

fn install(directory: &Path, release: &str, city_gzip: &[u8], asn_gzip: &[u8]) -> Result<()> {
    install_one(directory, CITY_FILE, city_gzip)?;
    install_one(directory, ASN_FILE, asn_gzip)?;
    let marker_tmp = directory.join(format!("{RELEASE_FILE}.tmp"));
    fs::write(&marker_tmp, release)?;
    fs::rename(marker_tmp, directory.join(RELEASE_FILE))?;
    GeoIpDatabase::open(directory).context("validate downloaded GeoIP databases")?;
    Ok(())
}

fn install_one(directory: &Path, filename: &str, compressed: &[u8]) -> Result<()> {
    let mut decoder = GzDecoder::new(compressed);
    let mut contents = Vec::new();
    decoder.read_to_end(&mut contents)?;
    if contents.is_empty() {
        bail!("downloaded {filename} was empty");
    }
    let temporary = directory.join(format!("{filename}.tmp"));
    fs::write(&temporary, contents)?;
    fs::rename(temporary, directory.join(filename))?;
    Ok(())
}

fn current_release() -> String {
    let now = OffsetDateTime::now_utc();
    format!("{:04}-{:02}", now.year(), month_number(now.month()))
}

const fn month_number(month: Month) -> u8 {
    month as u8
}

#[cfg(test)]
mod tests {
    use super::is_public_ip;

    #[test]
    fn distinguishes_public_and_private_addresses() {
        assert!(is_public_ip("136.243.31.96".parse().unwrap()));
        assert!(!is_public_ip("192.168.7.20".parse().unwrap()));
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("fd00::1".parse().unwrap()));
    }
}
