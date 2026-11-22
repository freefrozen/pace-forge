//! 真实网络下的路网来源探测（默认不跑，避免 CI 依赖外网）。
//!
//!     cargo test --test live_roadnet -- --ignored --nocapture
//!
//! 三段：
//! 1. Overpass（竞速）
//! 2. 高德瓦片（tileroad 的整条链路）
//! 3. 单张瓦片的裸请求（复刻 fetch_tile 的 URL/UA/Referer，看清是「网络不通」
//!    还是「内容过短/空白瓦片」）

use std::time::Instant;

fn probe_tile_raw(lat: f64, lng: f64) {
    // 与 track/tileroad.rs 里的 ZOOM 保持一致
    let zoom: u32 = 17;
    let n = (1u32 << zoom) as f64;
    let x = ((lng + 180.0) / 360.0 * n).floor() as i32;
    let r = lat.to_radians();
    let y = ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n).floor() as i32;
    println!("[裸测] 中心 tile z={zoom} x={x} y={y}");
    let hosts = [
        "https://webrd01.is.autonavi.com/appmaptile?lang=zh_cn&size=1&scale=1&style=8",
        "https://webrd02.is.autonavi.com/appmaptile?lang=zh_cn&size=1&scale=1&style=8",
    ];
    for host in hosts {
        let url = format!("{host}&x={x}&y={y}&z={zoom}");
        let t = Instant::now();
        let r = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 (compatible; pace-forge/0.1)")
            .set("Referer", "https://www.amap.com/")
            .timeout(std::time::Duration::from_secs(8))
            .call();
        match r {
            Ok(resp) => {
                let mut buf = Vec::new();
                use std::io::Read;
                let ok = resp.into_reader().read_to_end(&mut buf).is_ok();
                println!(
                    "  {:<40} {:.2}s {} bytes{} decode={}",
                    &host[8..40],
                    t.elapsed().as_secs_f64(),
                    buf.len(),
                    if ok { "" } else { " [读失败]" },
                    image::load_from_memory(&buf).is_ok()
                );
            }
            Err(e) => println!("  {:<40} {:.2}s FAIL {e}", &host[8..40], t.elapsed().as_secs_f64()),
        }
    }
}

#[test]
#[ignore]
fn live_roadnet_probe() {
    // 城市校园尺度 bbox（约 180m × 120m，有路可画）
    let bbox = (39.9920, 116.3260, 39.9931, 116.3277);
    println!("bbox = {bbox:?}\n");

    let t0 = Instant::now();
    let osm = pace_forge::track::roadnet::fetch(bbox);
    match &osm {
        Ok(n) => println!(
            "[overpass] OK  {:.2}s 来源={} 段数={}",
            t0.elapsed().as_secs_f64(),
            n.source,
            n.segments.len()
        ),
        Err(e) => println!("[overpass] FAIL {:.2}s {e}", t0.elapsed().as_secs_f64()),
    }

    let t1 = Instant::now();
    let tiles = pace_forge::track::tileroad::fetch_roadnet(bbox);
    match &tiles {
        Ok(n) => println!(
            "[tiles]    OK  {:.2}s 来源={} 段数={}",
            t1.elapsed().as_secs_f64(),
            n.source,
            n.segments.len()
        ),
        Err(e) => println!("[tiles]    FAIL {:.2}s {e}", t1.elapsed().as_secs_f64()),
    }

    println!();
    probe_tile_raw(39.9925, 116.3268);
    println!(
        "\n结论：overpass={} tiles={}",
        if osm.is_ok() { "可用" } else { "不可用" },
        if tiles.is_ok() { "可用" } else { "不可用" }
    );
}
