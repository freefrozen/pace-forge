//! 账号级风控体检 + 刷跑节流建议。
//!
//! 服务端抓人抓的不是「你用了什么工具」，而是**记录之间的同源特征**：
//! 连续 N 天准点、里程恒定、配速一条线、轨迹重合。2026-11-19 学校通报的
//! 「连续 20 多天都是脚本」就是这类特征堆出来的。
//!
//! 这个模块做两件事：
//!
//! 1. [`assess`]：把你自己的历史记录当成一份「体检表」，算连续天数、时刻
//!    规律性、里程/配速离散度、同日多条等指标，给一个 0~100 的风险分和
//!    具体扣分项——这是**本地自查**，不需要任何特权接口。
//! 2. [`suggest`]：按「本周次数 / 距上次间隔」决定今天该不该刷，并在不越
//!    日上限的前提下给出与最近记录**有差异**的参数。
//!
//! 另有一条服务端自查：`cheat/cheatlist` 接口会返回自己那一栏（见
//! `api::cheat`），那是唯一能直接确认「有没有被标记」的通道，两个一起用。

use serde_json::Value;

use super::records;

/// 一条历史记录的轻量视图。
#[derive(Debug, Clone, Copy)]
pub struct Run {
    pub start_ms: i64,
    pub dis_m: f64,
    pub dur_sec: f64,
    /// 平均步频（步/分）。
    pub cadence: f64,
}

impl Run {
    pub fn pace_sec_per_km(&self) -> f64 {
        if self.dis_m > 1.0 {
            self.dur_sec / (self.dis_m / 1000.0)
        } else {
            0.0
        }
    }
    pub fn hour(&self) -> i64 {
        // 服务器时间是 UTC+8 的毫秒戳；小时够用了，不做时区库
        ((self.start_ms / 3_600_000) + 8) % 24
    }
    pub fn day_key(&self) -> i64 {
        (self.start_ms + 8 * 3_600_000) / 86_400_000
    }
}

/// 把 `records::list_records` 的返回解析成 `Run`。
pub fn parse_runs(v: &Value) -> Vec<Run> {
    let rows = v
        .get("records")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Run> = rows
        .iter()
        .filter_map(|r| {
            let start_ms = r.get("startTime")?.as_i64()?;
            let dis_m = r
                .get("totalDistance")
                .or_else(|| r.get("totalDis"))?
                .as_f64()?;
            let dur_sec = r.get("totalTime")?.as_f64()?;
            let cadence = r
                .get("avgStepFreq")
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0);
            Some(Run { start_ms, dis_m, dur_sec, cadence })
        })
        .collect();
    out.sort_by_key(|r| r.start_ms);
    out
}

/// 体检结论。
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// 0~100，越高越像「机器排班」。
    pub score: f64,
    /// 连续跑步天数（最长的一段）。
    pub streak_days: i64,
    /// 最近 7 天跑了几天。
    pub days_last_7: i64,
    /// 时段集中度：最多的那个小时占全部记录的比例。
    pub hour_concentration: f64,
    /// 那个「最多的小时」。
    pub top_hour: i64,
    /// 里程的变异系数（标准差 / 均值）。
    pub dist_cv: f64,
    /// 配速的变异系数。
    pub pace_cv: f64,
    /// 同一天出现多条记录的天数。
    pub same_day_days: i64,
    /// 日均里程（有记录的天）。
    pub mean_km: f64,
    /// 具体问题，按严重度排。
    pub flags: Vec<(u32, String)>,
}

impl Report {
    pub fn grade(&self) -> &'static str {
        match self.score {
            s if s < 25.0 => "低",
            s if s < 55.0 => "中",
            _ => "高",
        }
    }
}

fn cv(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    if mean.abs() < 1e-9 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt() / mean
}

/// 体检：输入按时间升序的历史记录。
pub fn assess(runs: &[Run], now_ms: i64) -> Report {
    let mut rep = Report::default();
    if runs.is_empty() {
        return rep;
    }
    let day = |ms: i64| (ms + 8 * 3_600_000) / 86_400_000;

    // 连续天数：按自然日去重后找最长连续段，且必须一直连到今天/昨天
    let mut days: Vec<i64> = runs.iter().map(|r| r.day_key()).collect();
    days.sort_unstable();
    days.dedup();
    let mut best = 0;
    let mut cur = 0;
    let mut prev: Option<i64> = None;
    for d in &days {
        cur = match prev {
            Some(p) if *d == p + 1 => cur + 1,
            _ => 1,
        };
        best = best.max(cur);
        prev = Some(*d);
    }
    rep.streak_days = best;

    // 最近 7 天跑了几次（按天去重）
    let today = day(now_ms);
    let mut last7: Vec<i64> = days.iter().copied().filter(|d| today - d < 7).collect();
    last7.dedup();
    rep.days_last_7 = last7.len() as i64;

    // 时段集中度
    let mut hours = [0i64; 24];
    for r in runs {
        hours[r.hour() as usize] += 1;
    }
    let (top_hour, top_n) = hours
        .iter()
        .enumerate()
        .max_by_key(|(_, &n)| n)
        .map(|(h, &n)| (h as i64, n))
        .unwrap_or((0, 0));
    rep.top_hour = top_hour;
    rep.hour_concentration = top_n as f64 / runs.len() as f64;

    // 里程/配速离散度（只看 >=1km 的有效记录）
    let km: Vec<f64> = runs
        .iter()
        .filter(|r| r.dis_m >= 1000.0)
        .map(|r| r.dis_m / 1000.0)
        .collect();
    let pace: Vec<f64> = runs
        .iter()
        .filter(|r| r.dis_m >= 1000.0)
        .map(|r| r.pace_sec_per_km())
        .filter(|p| *p > 0.0)
        .collect();
    rep.dist_cv = cv(&km);
    rep.pace_cv = cv(&pace);
    rep.mean_km = if km.is_empty() { 0.0 } else { km.iter().sum::<f64>() / km.len() as f64 };

    // 同日多条
    let mut per_day: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for r in runs {
        *per_day.entry(r.day_key()).or_insert(0) += 1;
    }
    rep.same_day_days = per_day.values().filter(|&&n| n > 1).count() as i64;

    // ---- 打分 ----
    let mut score = 0.0f64;
    let mut flags: Vec<(u32, String)> = Vec::new();

    if rep.streak_days >= 10 {
        score += 35.0;
        flags.push((35, format!("连续 {} 天都有记录——学校通报的形态就是这个数", rep.streak_days)));
    } else if rep.streak_days >= 5 {
        score += 20.0;
        flags.push((20, format!("连续 {} 天都有记录，建议中间断开一天", rep.streak_days)));
    }

    if rep.days_last_7 >= 5 {
        score += 20.0;
        flags.push((20, format!("最近 7 天跑了 {} 天，频率偏高（真人 2~3 天）", rep.days_last_7)));
    } else if rep.days_last_7 >= 4 {
        score += 8.0;
        flags.push((8, format!("最近 7 天跑了 {} 天", rep.days_last_7)));
    }

    if rep.hour_concentration >= 0.6 && runs.len() >= 4 {
        score += 15.0;
        flags.push((15, format!(
            "{} 点这个小时占了 {:.0}% 的记录，时刻太规律",
            rep.top_hour,
            rep.hour_concentration * 100.0
        )));
    }

    if rep.dist_cv < 0.08 && km.len() >= 4 {
        score += 15.0;
        flags.push((15, format!(
            "每次里程几乎一样（变异系数 {:.1}%），真人会有 2.0/2.6/2.4 这种起伏",
            rep.dist_cv * 100.0
        )));
    }

    if rep.pace_cv < 0.05 && pace.len() >= 4 {
        score += 10.0;
        flags.push((10, format!(
            "配速波动只有 {:.1}%，像同一条流水线",
            rep.pace_cv * 100.0
        )));
    }

    if rep.same_day_days >= 2 {
        score += 10.0;
        flags.push((10, format!("有 {} 天出现过两次以上记录", rep.same_day_days)));
    }

    flags.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    rep.flags = flags;
    rep.score = score.min(100.0);
    rep
}

/// 今天的刷跑建议。
#[derive(Debug, Clone)]
pub struct Suggest {
    /// 今天还能不能刷。
    pub allowed: bool,
    /// 不能刷的原因。
    pub reason: String,
    /// 建议里程（公里）。
    pub km: f64,
    /// 建议时长（分钟）。
    pub minutes: i64,
    /// 建议起跑时刻（小时）。
    pub hour: i64,
}

/// 按历史给今天的参数建议。
///
/// 规则（都能从数据里推，不靠猜）：
/// - 今天已有记录 → 不再刷（日上限也不够）
/// - 最近 7 天已刷 ≥3 天 → 建议隔一天
/// - 里程从 2.0~2.8 里挑一个与最近 3 条**明显不同**的值
/// - 时长 21~26 分钟（学校要求「21 分钟以上」）
/// - 时刻避开历史最集中的那个小时
pub fn suggest(runs: &[Run], now_ms: i64, seed: u64) -> Suggest {
    let day = |ms: i64| (ms + 8 * 3_600_000) / 86_400_000;
    let today = day(now_ms);
    let rep = assess(runs, now_ms);

    if runs.iter().any(|r| r.day_key() == today) {
        return Suggest {
            allowed: false,
            reason: "今天已经有记录了，日上限 3 公里也基本用掉了".into(),
            km: 0.0,
            minutes: 0,
            hour: 0,
        };
    }
    if rep.days_last_7 >= 3 {
        return Suggest {
            allowed: false,
            reason: format!("最近 7 天已跑 {} 天，建议今天断一天再说", rep.days_last_7),
            km: 0.0,
            minutes: 0,
            hour: 0,
        };
    }

    // 里程：从候选里挑离最近几条最远的
    let recent: Vec<f64> = runs
        .iter()
        .rev()
        .take(3)
        .map(|r| r.dis_m / 1000.0)
        .collect();
    let cands = [2.0, 2.2, 2.4, 2.6, 2.8];
    let mut best = (cands[0], -1.0f64);
    for c in cands {
        let min_gap = recent
            .iter()
            .map(|r| (r - c).abs())
            .fold(f64::INFINITY, f64::min);
        let gap = if min_gap.is_finite() { min_gap } else { 9.9 };
        if gap > best.1 {
            best = (c, gap);
        }
    }
    let km = best.0;

    // 时长：让配速落在 7'00"~8'30"/km 之间
    let mut minutes = ((km * 1000.0 * 7.0 * 60.0) / 60_000.0).round() as i64; // 7'00"
    let slow = ((km * 1000.0 * 8.5 * 60.0) / 60_000.0).round() as i64; // 8'30"
    minutes = minutes.clamp(21, slow.max(21));
    // 用 seed 在 [min, slow] 里挑一个，避开整数小时的整齐感
    if slow > minutes {
        let span = (slow - minutes + 1) as u64;
        minutes += (seed % span) as i64;
    }

    // 时刻：避开最集中的小时
    let hour = {
        let mut h = (7 + (seed >> 8) % 12) as i64; // 7~18 起步
        if (h - rep.top_hour).abs() <= 1 {
            h = (h + 5) % 24;
        }
        h.clamp(16, 21)
    };

    Suggest {
        allowed: true,
        reason: format!(
            "最近 7 天 {} 天、连续 {} 天；建议与最近 3 条（{}）拉开差异",
            rep.days_last_7,
            rep.streak_days,
            recent
                .iter()
                .map(|x| format!("{x:.1}"))
                .collect::<Vec<_>>()
                .join("/")
        ),
        km,
        minutes,
        hour,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(day_offset: i64, hour: i64, km: f64, dur_min: f64) -> Run {
        let base = 1_764_000_000_000i64; // 2025-11 左右，只用于相对比较
        Run {
            start_ms: base + day_offset * 86_400_000 + hour * 3_600_000,
            dis_m: km * 1000.0,
            dur_sec: dur_min * 60.0,
            cadence: 100.0,
        }
    }

    #[test]
    fn streak_and_flags_detect_machine_schedule() {
        // 连跑 12 天、同一时刻、里程一样
        let runs: Vec<Run> = (0..12).map(|i| run(i, 18, 3.0, 21.0)).collect();
        let rep = assess(&runs, runs.last().unwrap().start_ms);
        assert_eq!(rep.streak_days, 12);
        assert!(rep.score >= 55.0, "连跑 12 天同点同里程，分数应偏高: {}", rep.score);
        assert!(rep.flags.iter().any(|(_, s)| s.contains("连续 12 天")));
    }

    #[test]
    fn varied_history_scores_low() {
        let specs = [
            (0, 19, 3.0, 16.0),
            (2, 17, 2.1, 14.0),
            (5, 20, 2.6, 18.0),
            (9, 7, 2.4, 22.0),
            (13, 18, 3.0, 17.0),
        ];
        let runs: Vec<Run> = specs
            .iter()
            .map(|(d, h, km, m)| run(*d, *h, *km, *m))
            .collect();
        let rep = assess(&runs, runs.last().unwrap().start_ms);
        assert!(rep.score < 25.0, "有意起伏的历史应低分: {}", rep.score);
    }

    #[test]
    fn suggest_refuses_when_ran_today() {
        let runs = vec![run(0, 18, 3.0, 22.0)];
        let s = suggest(&runs, runs[0].start_ms, 42);
        assert!(!s.allowed);
    }

    #[test]
    fn suggest_varies_distance_from_recent() {
        let runs = vec![run(-2, 18, 2.4, 22.0), run(-5, 19, 2.4, 22.0)];
        let s = suggest(&runs, 1_764_000_000_000 + 10 * 86_400_000, 7);
        assert!(s.allowed);
        assert!((s.km - 2.4).abs() > 0.15, "应避开 2.4，实际 {}", s.km);
        assert!(s.minutes >= 21);
    }
}

// ---------------------------------------------------------------------------
// 轨迹重合度
//
// 为什么需要它：在「全校每天都跑 3 公里」这种人群基线下，里程/时长/连续天数
// 都不再是区分特征——区分点是**每条轨迹是不是同一根线**。脚本生成的路由由
// 同一套算法 + 同一批点位决定，连日跑出来几乎重合；真人即使每天同一条路线，
// 拐弯、绕人、进出门的位置每次都不一样（实测同一条 400m 跑道，两次跑的
// 采样点平均偏差在 8~20 米）。
//
// 做法：把一条轨迹沿弧长重采样成定长签名（默认 40 点），比较时算「对应点
// 平均距离」，换算成 0~100 的重合度。不依赖起点方向（两条都试一次，取更
// 相似的那个方向）。
// ---------------------------------------------------------------------------

/// 从 OBS / 详情返回里抠出经纬度序列。
///
/// 兼容三种形态：`points` 数组（`lat/lng` 或 `gLat/gLng`）、`allLocJson`
/// 字符串（内部是同样的数组）、以及裸数组。
/// OBS 对象里的字段是 gzip + base64 的字符串（`"run_data": "H4sIAAAA..."`），
/// 直接 `as_array()` 拿到的是个字符串，必须解一层。
fn gunzip_b64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    use std::io::Read as _;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()?;
    let mut d = flate2::read::GzDecoder::new(&raw[..]);
    let mut out = Vec::new();
    d.read_to_end(&mut out).ok()?;
    Some(out)
}

pub fn extract_points(v: &Value) -> Vec<(f64, f64)> {
    fn from_arr(a: &[Value]) -> Vec<(f64, f64)> {
        a.iter()
            .filter_map(|p| {
                let lat = p
                    .get("lat")
                    .or_else(|| p.get("gLat"))
                    .or_else(|| p.get("glat"))
                    .and_then(|x| x.as_f64())?;
                let lon = p
                    .get("lon")
                    .or_else(|| p.get("lng"))
                    .or_else(|| p.get("gLng"))
                    .or_else(|| p.get("glon"))
                    .and_then(|x| x.as_f64())?;
                if lat.abs() < 1e-6 && lon.abs() < 1e-6 {
                    return None;
                }
                Some((lat, lon))
            })
            .collect()
    }
    if let Some(a) = v.as_array() {
        return from_arr(a);
    }
    // OBS 明细：点位在 `run_data`（压缩串）里，顺序排在裸数组之前。
    for key in ["run_data", "points", "allLocJson", "locs", "track"] {
        match v.get(key) {
            Some(Value::Array(a)) => {
                let got = from_arr(a);
                if !got.is_empty() {
                    return got;
                }
            }
            Some(Value::String(s)) => {
                if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(s) {
                    let got = from_arr(&a);
                    if !got.is_empty() {
                        return got;
                    }
                }
                // gzip + base64（OBS 的真实形态）。解出来可能是裸数组，
                // 也可能是 `{"points": [...]}` 这种包了一层的对象。
                if let Some(bytes) = gunzip_b64(s) {
                    if let Ok(vv) = serde_json::from_slice::<Value>(&bytes) {
                        let got = match &vv {
                            Value::Array(a) => from_arr(a),
                            Value::Object(o) => ["points", "locs", "allLocJson", "track", "data"]
                                .iter()
                                .find_map(|k| match o.get(*k) {
                                    Some(Value::Array(a)) => Some(from_arr(a)),
                                    Some(Value::String(t)) => {
                                        serde_json::from_str::<Value>(t).ok().and_then(|x| {
                                            x.as_array().map(|a| from_arr(a))
                                        })
                                    }
                                    _ => None,
                                })
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        if !got.is_empty() {
                            return got;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

fn arc_length(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2)
        .map(|w| crate::track::geo::distance(w[0], w[1]))
        .sum()
}

/// 沿弧长等距重采样成 `n` 个点（保留起终点）。
pub fn signature(pts: &[(f64, f64)], n: usize) -> Vec<(f64, f64)> {
    if pts.len() < 2 || n < 4 {
        return pts.to_vec();
    }
    let total = arc_length(pts);
    if total <= 1.0 {
        return pts.to_vec();
    }
    let step = total / (n - 1) as f64;
    let mut out = Vec::with_capacity(n);
    // 起点先放一个，随后 target 从「一个步长」开始——早先 target 从 0 起，
    // 每个段都会再 push 一次起点（重复点），40 点里有一个是无效的，
    // 正反向比对因此对不齐（`reversed_track_still_matches` 就是这么挂的）。
    let mut target = step;
    let mut acc = 0.0f64;
    out.push(pts[0]);
    for w in pts.windows(2) {
        let seg = crate::track::geo::distance(w[0], w[1]);
        while acc + seg >= target && out.len() < n {
            let t = if seg > 1e-9 { (target - acc) / seg } else { 0.0 };
            out.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
            target += step;
        }
        acc += seg;
        if out.len() >= n {
            break;
        }
    }
    while out.len() < n {
        out.push(*pts.last().unwrap());
    }
    out
}

/// 两条轨迹的重合度（0~1）。1 = 完全重合。
///
/// 换算：对应点平均距离 0m → 100%；平均 40m 以上 → 0%。
pub fn similarity(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let n = 40usize;
    if a.len() < 4 || b.len() < 4 {
        return 0.0;
    }
    let sa = signature(a, n);
    let mut sb = signature(b, n);
    let mean_dist = |x: &[(f64, f64)], y: &[(f64, f64)]| -> f64 {
        let mut sum = 0.0;
        for i in 0..x.len().min(y.len()) {
            sum += crate::track::geo::distance(x[i], y[i]);
        }
        sum / x.len().min(y.len()) as f64
    };
    let d1 = mean_dist(&sa, &sb);
    sb.reverse();
    let d2 = mean_dist(&sa, &sb);
    let d = d1.min(d2);
    (1.0 - (d / 40.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod track_tests {
    use super::*;

    fn line(dx: f64) -> Vec<(f64, f64)> {
        (0..=100)
            .map(|i| (34.0 + i as f64 * 1e-5, 113.0 + dx))
            .collect()
    }

    #[test]
    fn identical_tracks_score_one() {
        let a = line(0.0);
        assert!(similarity(&a, &a) > 0.99);
    }

    #[test]
    fn shifted_tracks_score_low() {
        // 同样的形状、整体偏移约 60m
        let a = line(0.0);
        let b = line(0.0008);
        let s = similarity(&a, &b);
        assert!(s < 0.1, "偏移 60m 应几乎不重合，实际 {s}");
    }

    #[test]
    fn reversed_track_still_matches() {
        let a = line(0.0);
        let mut b = a.clone();
        b.reverse();
        assert!(similarity(&a, &b) > 0.9, "反着走同一条线也应判重合");
    }

    #[test]
    fn signature_keeps_endpoints() {
        let a = line(0.0);
        let s = signature(&a, 40);
        assert_eq!(s.len(), 40);
        assert!((s[0].0 - a[0].0).abs() < 1e-12);
    }
}
