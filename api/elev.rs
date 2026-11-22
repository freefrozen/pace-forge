//! 海拔真值：按定位锚点查数字高程模型（DEM），供 `bdA` / `endAltAbs` 用。
//!
//! 为什么必须有这个模块：`api/submit.rs` 里 `bdA` 的默认值是 `150 ± 8m`，
//! 取自唯一那条真机记录（齐齐哈尔，平原校园，实测 142.4~158.5m）。这个默认
//! 值一旦用在别处就自相矛盾——许昌学院实测 67~70m，成都约 500m，昆明约
//! 1890m。服务端手里有坐标，拿坐标反查高程差出几十米到上千米，比配速异常
//! 硬得多：配速还能解释成天赋，海拔和坐标对不上没有任何解释空间。
//!
//! 数据源用 open-elevation（公开、无需 key）。查询结果按**取整后的坐标**
//! 缓存在数据目录里，同一个校区只查一次，之后离线可用；查不到就回落到
//! `None`，由调用方决定退回默认值——绝不因为网络失败而把整条流程打断。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// 公开高程查询服务（按顺序试）。都走 SRTM 90m 网格，平原校园精度足够。
///
/// 为什么要两个：2026-11-18 实测 open-elevation 单点响应偶发 19 秒、且
/// rustls 侧出现 `tls connection init failed (os error 35)`——单源会直接把
/// 整条流程拖死。open-meteo 的 `elevation` 接口返回同样量级的 SRTM 值，
/// 换源比加重试更快。
const PROVIDERS: [&str; 2] = [
    "https://api.open-elevation.com/api/v1/lookup",
    "https://api.open-meteo.com/v1/elevation",
];

/// 数据目录下的缓存文件名。
const CACHE_NAME: &str = "elev-cache.json";

/// `bdA` 的半宽（米）。
///
/// 真机那条记录的 `bdA` 跨度是 142.4~158.5（±8m 上下的传感器噪声，不是地形），
/// 所以区间half宽度沿用 8，只把**基准**换成 DEM 真值。
const HALF_M: f64 = 8.0;

fn cache_path() -> PathBuf {
    super::session::data_dir().join(CACHE_NAME)
}

/// 缓存 key：坐标取到小数点后 3 位（约 100m），够用且不会把同一校区算成多个键。
fn key_of(lat: f64, lon: f64) -> String {
    format!("{lat:.3},{lon:.3}")
}

fn load_cache() -> HashMap<String, f64> {
    std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|o| {
            o.into_iter()
                .filter_map(|(k, v)| v.as_f64().map(|f| (k, f)))
                .collect()
        })
        .unwrap_or_default()
}

fn save_cache(cache: &HashMap<String, f64>) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(&path, s);
    }
}

/// 环境变量覆盖：`PACE_ALT_RANGE="67,71"`。
///
/// 给「已经知道本地海拔、不想再查网」的场景用（也是离线环境的唯一入口）。
pub fn from_env() -> Option<(f64, f64)> {
    let raw = std::env::var("PACE_ALT_RANGE").ok()?;
    let mut it = raw.split(',').map(|s| s.trim().parse::<f64>());
    let lo = it.next()?.ok()?;
    let hi = it.next()?.ok()?;
    if hi > lo + 1.0 {
        Some((lo, hi))
    } else {
        None
    }
}

/// 查锚点高程，返回可直接喂给 `SubmitParams::alt_range` 的区间。
///
/// 顺序：内存/磁盘缓存 → 在线查询 → `None`（调用方退回默认值）。
pub fn range_for(lat: f64, lon: f64) -> Option<(f64, f64)> {
    let base = lookup(lat, lon)?;
    Some((base - HALF_M, base + HALF_M))
}

/// 手工写入缓存（米）。网络查不到时用这个填一次，之后离线可用。
pub fn remember(lat: f64, lon: f64, base: f64) -> bool {
    if !base.is_finite() || !(0.0..=9000.0).contains(&base) {
        return false;
    }
    let mut cache = load_cache();
    cache.insert(key_of(lat, lon), base);
    save_cache(&cache);
    true
}

/// 只查基准高程（米）。命中缓存时不发请求。
pub fn lookup(lat: f64, lon: f64) -> Option<f64> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let key = key_of(lat, lon);
    let mut cache = load_cache();
    if let Some(v) = cache.get(&key) {
        return Some(*v);
    }
    let v = query(lat, lon).ok()?;
    cache.insert(key, v);
    save_cache(&cache);
    Some(v)
}

fn query(lat: f64, lon: f64) -> Result<f64, String> {
    let mut last = String::from("没有可用端点");
    for url in PROVIDERS {
        match query_one(url, lat, lon) {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// 单个端点：`open-elevation` 走 POST JSON，`open-meteo` 走 GET 查询串。
fn query_one(url: &str, lat: f64, lon: f64) -> Result<f64, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(12))
        .build();
    if url.contains("open-meteo") {
        let resp = agent
            .get(&format!("{url}?latitude={lat}&longitude={lon}"))
            .call()
            .map_err(|e| format!("{url}: {e}"))?;
        let v: Value = resp.into_json().map_err(|e| format!("{url}: {e}"))?;
        return v
            .get("elevation")
            .and_then(|e| e.get(0))
            .and_then(|e| e.as_f64())
            .ok_or_else(|| format!("{url}: 响应里没有 elevation"));
    }
    let body = json!({ "locations": [{ "latitude": lat, "longitude": lon }] });
    let resp = agent
        .post(url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| format!("{url}: {e}"))?;
    let v: Value = resp.into_json().map_err(|e| format!("{url}: {e}"))?;
    v.get("results")
        .and_then(|r| r.get(0))
        .and_then(|r| r.get("elevation"))
        .and_then(|e| e.as_f64())
        .ok_or_else(|| format!("{url}: 响应里没有 elevation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_rounds_to_100m() {
        // 3 位小数 = 约 100m 网格：同一格里的两点必须同键
        assert_eq!(key_of(34.04812, 113.87611), key_of(34.04849, 113.87649));
        assert_ne!(key_of(34.048, 113.876), key_of(34.060, 113.876));
    }

    #[test]
    fn rejects_impossible_coordinates() {
        assert_eq!(lookup(0.0, -999.0), None);
        assert_eq!(lookup(91.0, 0.0), None);
    }

    #[test]
    fn env_override_parses_pair() {
        std::env::set_var("PACE_ALT_RANGE", "67,71");
        assert_eq!(from_env(), Some((67.0, 71.0)));
        std::env::set_var("PACE_ALT_RANGE", "71,67");
        assert_eq!(from_env(), None);
        std::env::remove_var("PACE_ALT_RANGE");
    }
}

#[cfg(test)]
mod dbg {
    #[test]
    fn dbg_query() {
        println!("{:?}", super::query(34.04851, 113.87699));
    }
}
