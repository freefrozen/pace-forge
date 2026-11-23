//! 提交前硬拦截：三类「参数不自洽」的检查。
//!
//! 起因是 2026-11-22 那条被驳回的记录（rrid 1368728975）：服务端审核意见写
//! **「APP 参数数据异常 201」**，`reasonList` 里前六项规则全过，只有
//! 「运动数据有问题」没过。同一天之后提交的那条（海拔/起步已修）是
//! `status=0`。也就是说：**它们的判据不在规则里，在参数自洽上**。
//!
//! 这里把已经知道的三条做成硬拦截，宁可本地失败，也不要把注定被驳回的
//! 报文发出去——驳回会记错误次数，累计到阈值就是账号处理。

use super::elev;
use super::flow::RunParams;
use crate::track::Checkpoint;

/// 海拔基准与锚点实测高程的容差（米）。
///
/// 真机那条样本的 `bdA` 跨度是 ±8m（传感器噪声）；这里放宽到 20m，
/// 只拦「一个在平原、一个在高原」这种量级，不误伤城市内的正常起伏。
pub const ALT_TOLERANCE_M: f64 = 20.0;

/// 起步窗口上限（秒）。真机实测 86 秒；超过这个数就没有真机样本支撑了。
pub const STARTUP_LIMIT_SEC: i64 = 90;

/// 一条拦截结果。
#[derive(Debug, Clone)]
pub struct Verdict {
    pub blocking: Vec<String>,
    pub warnings: Vec<String>,
}

impl Verdict {
    pub fn blocked(&self) -> bool {
        !self.blocking.is_empty()
    }
}

/// 检查并打印。返回 `Err` 表示这条不该发。
pub fn inspect(
    p: &RunParams,
    alt_range: Option<(f64, f64)>,
    cps: &[Checkpoint],
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let mut v = Verdict {
        blocking: Vec::new(),
        warnings: Vec::new(),
    };

    // ---- ① 海拔与坐标的一致性 ----
    //
    // 2026-11-18 那条就是死在这里：坐标在某校区（实测高程 69m），报文写
    // 150±8m（齐齐哈尔样本），差 80 米。服务端拿坐标反查高程就能判定，
    // 不需要任何模型。
    let base_used = alt_range.map(|(lo, hi)| (lo + hi) / 2.0);
    let dem = elev::lookup(p.center_lat, p.center_lng);
    match (base_used, dem) {
        (Some(b), Some(d)) if (b - d).abs() > ALT_TOLERANCE_M => {
            v.blocking.push(format!(
                "海拔基准 {b:.0}m 与锚点实测高程 {d:.0}m 相差 {:.0}m（容差 {ALT_TOLERANCE_M:.0}m）——服务端拿坐标反查高程即可判定「参数异常」",
                (b - d).abs()
            ));
        }
        (Some(b), Some(d)) => log(&format!(
            "√ [guard] 海拔与坐标一致：{b:.0}m vs 实测 {d:.0}m"
        )),
        (None, Some(d)) => {
            v.warnings.push(format!(
                "没拿到海拔区间，会用默认 150±8m，而锚点实测高程是 {d:.0}m"
            ));
        }
        (_, None) => {
            v.warnings
                .push("查不到锚点高程（DEM 不可用），无法校验海拔与坐标一致性".into());
        }
    }

    // ---- ② 跑法与点位的一致性 ----
    //
    // `runMode=2`（户外跑）按协议不带点位。带上点位会被判「参数与模式不符」，
    // 这也属于 201 那一类。11-18 那条正是 runMode=2 + 5 个点位。
    if p.run_mode != 1 && !cps.is_empty() {
        v.blocking.push(format!(
            "跑法是 runMode={}（户外跑/室外锻炼），却带 {} 个打卡点——协议上这类跑法没有点位，服务端会判参数与模式不符",
            p.run_mode,
            cps.len()
        ));
    }

    // ---- ③ 起步窗口长度 ----
    //
    // 真机是 86 秒。按 15.5% 外推到 20 分钟记录会切出 186 秒，那 3 分钟里
    // 速度接近 0，逐点配速 30~90 分/公里，对「4'00"~12'00"」这种规则整段越界。
    let su = crate::track::startup_sec(p.duration_sec);
    if su > STARTUP_LIMIT_SEC {
        v.blocking.push(format!(
            "起步窗口 {su} 秒超过真机上限 {STARTUP_LIMIT_SEC} 秒（真机实测 86 秒），起步段配速会整段越界"
        ));
    } else {
        let share = su as f64 / p.duration_sec.max(1) as f64;
        log(&format!(
            "√ [guard] 起步窗口 {su} 秒（占时长 {:.1}%，真机 15.5% / 86 秒）",
            share * 100.0
        ));
    }

    for w in &v.warnings {
        log(&format!("! [guard] {w}"));
    }
    if v.blocked() {
        for b in &v.blocking {
            log(&format!("✗ [guard] {b}"));
        }
        return Err(format!(
            "参数自洽检查未通过（{} 项）：这条发出去大概率被驳回，且会记错误次数",
            v.blocking.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_dem() {
        // 测试不依赖网络，也不依赖真实地形：先把锚点高程写进缓存
        crate::api::elev::remember(30.50000, 104.00000, 69.0);
    }

    fn params(run_mode: i64, dur_sec: i64) -> RunParams {
        seed_dem();
        RunParams {
            center_lat: 30.50000,
            center_lng: 104.00000,
            distance_m: 2400.0,
            duration_sec: dur_sec,
            run_mode,
            ..Default::default()
        }
    }

    fn cps(n: usize) -> Vec<Checkpoint> {
        (0..n)
            .map(|i| Checkpoint {
                lat: 30.50 + i as f64 * 1e-4,
                lng: 104.00,
                glat: 30.50 + i as f64 * 1e-4,
                glon: 104.00,
                name: format!("P{i}"),
            })
            .collect()
    }

    #[test]
    fn blocks_altitude_far_from_dem() {
        seed_dem();
        // 150±8m 是齐齐哈尔的默认值，某校区实测 69m
        let mut log_lines = Vec::new();
        let r = inspect(&params(1, 1320), Some((142.0, 158.0)), &cps(5), &mut |s| {
            log_lines.push(s.to_string())
        });
        assert!(r.is_err(), "海拔差 80m 应该被拦截");
    }

    #[test]
    fn blocks_points_with_outdoor_mode() {
        seed_dem();
        let r = inspect(&params(2, 1320), None, &cps(5), &mut |_| {});
        assert!(r.is_err(), "runMode=2 带点位应该被拦截");
    }

    #[test]
    fn startup_window_blocked_unless_capped() {
        // 环境变量是进程级的，这两条必须写在同一个测试里，否则并行跑会互相抢
        seed_dem();
        unsafe { std::env::remove_var("PACE_STARTUP_CAP_SEC") };
        let r = inspect(&params(1, 1320), None, &cps(5), &mut |_| {});
        assert!(r.is_err(), "不封顶时 22 分钟记录的起步窗口是 205 秒，应被拦截");

        unsafe { std::env::set_var("PACE_STARTUP_CAP_SEC", "86") };
        let r = inspect(&params(1, 1320), None, &cps(5), &mut |_| {});
        assert!(r.is_ok(), "封顶 86 秒后应放行");
        unsafe { std::env::remove_var("PACE_STARTUP_CAP_SEC") };
    }
}
