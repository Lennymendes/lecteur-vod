//! Mesure de débit d'une URL googlevideo (diagnostic du bridage « n » : ~50–100 ko/s quand le
//! défi n'est pas résolu, plusieurs Mo/s sinon).

use std::io::Read;
use std::time::{Duration, Instant};

/// Télécharge jusqu'à `bytes` octets (requête Range) et renvoie (octets reçus, Mo/s).
pub fn throughput(url: &str, headers: &[(String, String)], bytes: u64) -> Result<(u64, f64), String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let mut req = client.get(url).header("Range", format!("bytes=0-{}", bytes - 1));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let t = Instant::now();
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let mut got = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    let mut body = resp;
    while got < bytes {
        match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got += n as u64,
            Err(e) if got > 0 => {
                let _ = e;
                break;
            }
            Err(e) => return Err(e.to_string()),
        }
        if t.elapsed() > Duration::from_secs(15) {
            break;
        }
    }
    let secs = t.elapsed().as_secs_f64().max(1e-3);
    Ok((got, got as f64 / secs / 1_048_576.0))
}
