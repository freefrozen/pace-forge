//! 生物力学：配速、步频、步幅的非线性耦合。
//!
//! 硬约束：`v = (cadence / 60) · stride`（v: m/s，cadence: 步/分，stride: 米）
//!
//! **这个约束必须始终成立**。早期实现里步频和步数是各自随机生成的，两者与
//! 速度对不上——服务端只要拿 `totalSteps / totalTime` 和 `totalDis / totalTime`
//! 一除就能看出矛盾：步频 192 步/分 × 步幅必然推出的速度，与报文里的配速不符。
//!
//! 步频公式由真实记录回归得到，不是拍脑袋常数。四条样本（配速 5:31 /
//! 6:40 / 7:02 / 8:19）回归出的斜率与截距，拟合误差 ≤2 步/分。

/// 标准正态抽样（Box-Muller）。
///
/// 不引 `rand_distr`：它要额外拉依赖，而这里只需要一个标准正态。
fn normal_sample(rng: &mut impl rand::Rng) -> f64 {
    let u1 = rng.gen::<f64>().max(1e-12);
    let u2 = rng.gen::<f64>();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// 步态：步频与步幅。
#[derive(Clone, Copy, Debug)]
pub struct Gait {
    /// 步频（步/分）。
    pub cadence: f64,
    /// 步幅（米）。
    pub stride: f64,
}

/// 回归的适用下界（m/s）。低于它不再用回归外推，改走真机慢跑锚点。
///
/// 回归样本是 2.0~3.0 m/s（5:31~8:19/km）四条真机记录；往下外推到
/// 12:00/km（1.39 m/s）会给出 96 步/分，被下限一夹就压成平带，步幅又被
/// 等式反推到 0.77m——真机实测是 179 步/分、0.47m（issue #7）。
pub const GAIT_REGRESSION_FLOOR_MPS: f64 = 2.0;

/// 由速度与疲劳度计算步态，严格满足 `v = cadence/60 * stride`。
///
/// 步频公式来自 4 条真实跑步记录的线性回归：`cadence = 34.4 + 44.5 * v`，
/// 拟合误差 ≤2 步/分。
///
/// | 配速 | 实测步频 | 公式预测 |
/// |------|---------|---------|
/// | 5:31/km (3.02 m/s) | 170 | 169 |
/// | 6:40/km (2.50 m/s) | 144 | 146 |
/// | 7:02/km (2.37 m/s) | 139 | 140 |
/// | 8:19/km (2.00 m/s) | 125 | 124 |
///
/// 常见的旧公式 `150 + 12v` 在 6:40/km 会给出 180 步/分，比实测高 36 步，
/// 而且形态是「步频主导」；真实数据是**步幅主导**——慢配速时步频掉得更多。
pub fn gait(v: f64, fatigue: f64) -> Gait {
    // 系数与上下限都可由环境变量覆盖（账号口径），默认值与上游一致。
    let (a, b) = coeffs();
    let (lo, hi) = cad_bounds();
    let smax = stride_max();

    if v >= GAIT_REGRESSION_FLOOR_MPS {
        let target_cad = (a + b * v) * (1.0 - 0.05 * fatigue);
        let mut cadence = target_cad.clamp(lo, hi);
        let mut stride = if cadence > 0.0 { v * 60.0 / cadence } else { 0.0 };

        // 步幅越界时反解步频，保持等式成立
        if stride > smax {
            stride = smax;
            cadence = v * 60.0 / stride;
        } else if stride < 0.35 {
            stride = 0.35;
            cadence = v * 60.0 / stride;
        }
        return Gait { cadence, stride };
    }

    // ---- 慢速走跑段（v < 2.0 m/s，即慢于 8:19/km）----
    //
    // 回归的下界是 2.0 m/s，**低于它不许外推**：拿回归去算 12:00/km
    // （1.39 m/s）会给出 96 步/分，被下限一夹就整段压成平带，步幅又被
    // 等式反推到 0.77m——issue #7 的实测对比正是这两个数（真机 179 /
    // 0.47m）。真机慢跑的形态是「步频顶在 175~185、步幅随速度缩」。
    //
    // 账号口径下这两个锚点要跟着缩放，否则会跟标定区间打架：设
    //
    //   k = (a + b·2.0) / 123.4      // 默认口径时 k=1，行为与上游逐位一致
    //
    // 于是「1.40 m/s 的步幅」从 0.47 放大成 0.47/k、上端锚点仍落在回归于
    // 2.0 的取值上，两段连续、形态不变、量级跟着账号走。
    let cad_top = a + b * GAIT_REGRESSION_FLOOR_MPS;
    let stride_top = if cad_top > 1.0 {
        GAIT_REGRESSION_FLOOR_MPS * 60.0 / cad_top
    } else {
        0.97
    };
    let k = (cad_top / 123.4).max(1e-6);
    let stride_low = (0.47 / k).clamp(0.30, stride_top);

    let stride0 = if v >= 1.40 {
        stride_low + (v - 1.40) * (stride_top - stride_low) / (2.0 - 1.40)
    } else {
        stride_low * v / 1.40
    }
    .clamp(0.0, stride_top);

    // 步频下限只防退化（v→0 时步幅缩到 0、等式仍成立），不夹 100：
    // 慢跑真机步频 179，夹 100 会把它压成一条平线。
    let mut cadence = (v * 60.0 / stride0.max(1e-9)) * (1.0 - 0.05 * fatigue);
    if cadence < 60.0 {
        cadence = 60.0;
    }
    let stride = if cadence > 0.0 { v * 60.0 / cadence } else { 0.0 };
    Gait { cadence, stride }
}

/// 步频公式系数（截距, 斜率）。
///
/// 默认值 `34.4 + 44.5 v` 是仓库作者用 4 条公开记录回归出来的「步态」口径。
/// **账号口径不一样**：App 自己算的 `avgStepFreq` 来自手机计步器（会少计），
/// 2026-11-18 拉某账号 20 条真实记录回归得到 `58.7 + 10.8 v`（n=18，剔除
/// 1.08km 短记录与 1.52 m/s 慢记录），中位步频 98.5、中位速度 3.29 m/s。
///
/// 同一个人历史平均 100 步/分，突然连续跑出 150 步/分，就是账号画像突变。
/// 所以把系数做成环境变量：
///
/// ```text
/// PACE_CAD_INTERCEPT=58.7 PACE_CAD_SLOPE=10.8 PACE_STRIDE_MAX=3.4
/// ```
pub fn coeffs() -> (f64, f64) {
    fn get(k: &str, d: f64) -> f64 {
        std::env::var(k)
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(d)
    }
    (get("PACE_CAD_INTERCEPT", 34.4), get("PACE_CAD_SLOPE", 44.5))
}

/// 步频上下限（步/分）。
///
/// 默认 `100..215` 是仓库旧口径（照着那 4 条样本）。一旦用账号口径覆盖了
/// 系数（`PACE_CAD_INTERCEPT`），说明目标是「像这个账号」，下限必须放到
/// 校内规则的 60——否则 100 的下限会把 98 拍成 100，反而抹掉账号特征。
pub fn cad_bounds() -> (f64, f64) {
    if std::env::var("PACE_CAD_INTERCEPT").is_ok() {
        (60.0, 300.0)
    } else {
        (100.0, 215.0)
    }
}

/// 步幅上限（米）。默认 1.7；账号口径下真实记录推出来能到 2.4~3.3，
/// 因为 App 的步数是少计的，`距离 ÷ 步数` 自然偏大。
pub fn stride_max() -> f64 {
    std::env::var("PACE_STRIDE_MAX")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.5)
        .unwrap_or(1.7)
}

/// 由速度与疲劳度取步幅（米）。
pub fn stride_for(v: f64, fatigue: f64) -> f64 {
    gait(v, fatigue).stride
}

/// 由速度与步幅反解步频（步/分）。
pub fn cadence_for(v: f64, stride: f64) -> f64 {
    (60.0 * v / stride.max(1e-6)).clamp(60.0, 300.0)
}

/// 由步频与步幅计算速度（m/s）。
pub fn speed_from(cadence: f64, stride: f64) -> f64 {
    (cadence / 60.0) * stride
}

/// 疲劳度（0..0.6）：已跑 0.5km 后开始线性抬升，封顶 0.6。
pub fn fatigue_from_km(km: f64) -> f64 {
    if km <= 0.5 {
        0.0
    } else {
        ((km - 0.5) * 1.5).min(0.6)
    }
}

/// 步频的 OU（Ornstein-Uhlenbeck）均值回归序列。
///
/// ```text
/// c_{t+1} = c_t + θ(target_t − c_t)·Δt + σ·√Δt·Z,   Z ~ N(0,1)
/// ```
///
/// 为什么不用均匀随机抖动：真实步频有**惯性**——既不会逐点乱跳，也不会
/// 一直贴在均值上，而是带着自相关地向目标回归。OU 过程正好描述这个，
/// 均匀随机数做不到（相邻点独立，序列看起来像白噪声）。
pub fn ou_cadence_series(
    targets: &[f64],
    dts: &[f64],
    seed: u64,
    theta: f64,
    sigma: f64,
    floor: f64,
    ceil: f64,
) -> Vec<f64> {
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(targets.len());
    let mut c = targets.first().copied().unwrap_or(150.0);

    for (i, &target) in targets.iter().enumerate() {
        let dt = dts.get(i).copied().unwrap_or(1.0);
        c += theta * (target - c) * dt + sigma * dt.sqrt() * normal_sample(&mut rng);
        c = c.clamp(floor, ceil);
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 步频必须匹配真实记录，而不是旧的 `150 + 12v`。
    #[test]
    fn cadence_matches_real_records() {
        for (pace_s, want) in [(331.0, 170.0), (400.0, 144.0), (422.0, 139.0), (499.0, 125.0)] {
            let v = 1000.0 / pace_s;
            let g = gait(v, 0.0);
            assert!(
                (g.cadence - want).abs() <= 3.0,
                "配速 {pace_s:.0}s/km 步频 {:.0} 偏离实测 {want} 超过 3 步",
                g.cadence
            );
            assert!(
                (0.85..=1.30).contains(&g.stride),
                "配速 {pace_s:.0}s/km 步幅 {:.2}m 超出真实跑者区间",
                g.stride
            );
        }
    }

    /// 等式约束必须始终成立，否则服务端一除就露馅。
    #[test]
    fn identity_always_holds() {
        for v in [1.5, 2.0, 2.5, 3.0, 3.5, 4.0] {
            for fatigue in [0.0, 0.5, 1.0] {
                let g = gait(v, fatigue);
                let back = g.cadence / 60.0 * g.stride;
                assert!(
                    (back - v).abs() < 1e-9,
                    "v={v} fatigue={fatigue} 重建速度 {back} 不符"
                );
            }
        }
    }

    #[test]
    fn gait_stays_in_envelope() {
        for v in [1.9f64, 2.5, 3.5, 5.0, 6.3] {
            for fat in [0.0f64, 0.3, 0.6] {
                let g = gait(v, fat);
                assert!((0.5..=1.7).contains(&g.stride), "stride={}", g.stride);
                assert!((110.0..=230.0).contains(&g.cadence), "cad={}", g.cadence);
                assert!((speed_from(g.cadence, g.stride) - v).abs() < 1e-6);
            }
        }
    }

    /// 慢速走跑段的锚点：12:00/km 实测步频 179、步幅 0.47m（issue #7 真机）。
    /// 回归外推在这里是错的：会给出 96 步/分、被下限夹成 100 出头的平线。
    #[test]
    fn slow_jog_matches_real_anchor() {
        let g = gait(1000.0 / 720.0, 0.0); // 12:00/km = 1.39 m/s
        assert!(
            (172.0..=185.0).contains(&g.cadence),
            "12:00/km 步频 {:.1} 应在 179 一带（真机实测），不是 100 出头",
            g.cadence
        );
        assert!(
            (0.42..=0.52).contains(&g.stride),
            "12:00/km 步幅 {:.3}m 应在 0.47 一带（真机实测）",
            g.stride
        );
        // 等式：步频×步幅必须能还原速度
        let back = g.cadence / 60.0 * g.stride;
        assert!((back - 1000.0 / 720.0).abs() < 1e-9, "等式被破坏：{back}");
    }

    /// 慢速分支与标定区间在 2.0 m/s 端点上连续：8:19/km 实测 125 步/分。
    #[test]
    fn slow_branch_continuous_at_regression_floor() {
        let below = gait(2.0 - 1e-9, 0.0);
        let at = gait(2.0, 0.0);
        assert!(
            (below.cadence - at.cadence).abs() < 1.0,
            "2.0 m/s 两侧步频 {:.1} vs {:.1} 应连续",
            below.cadence,
            at.cadence
        );
        assert!((at.cadence - 125.0).abs() <= 3.0, "8:19/km 实测 125");
        assert!(
            (below.stride - at.stride).abs() < 0.05,
            "2.0 m/s 两侧步幅 {:.3} vs {:.3} 应连续",
            below.stride,
            at.stride
        );
    }

    #[test]
    fn ou_series_is_bounded_and_deterministic() {
        let n = 200;
        let targets: Vec<f64> = (0..n).map(|i| 170.0 + 0.1 * i as f64).collect();
        let dts = vec![5.0; n];
        let a = ou_cadence_series(&targets, &dts, 42, 0.25, 0.5, 110.0, 220.0);
        assert_eq!(a.len(), n);
        for c in &a {
            assert!((110.0..=220.0).contains(c), "cadence={c}");
        }
        let b = ou_cadence_series(&targets, &dts, 42, 0.25, 0.5, 110.0, 220.0);
        assert_eq!(a, b, "同种子必须同结果");
    }

    /// OU 序列必须有自相关：相邻点差值的方差应远小于白噪声。
    #[test]
    fn ou_is_not_white_noise() {
        let n = 300;
        let targets = vec![160.0; n];
        let dts = vec![10.0; n];
        let s = ou_cadence_series(&targets, &dts, 7, 0.12, 0.8, 100.0, 220.0);
        let diffs: Vec<f64> = s.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        let mean_diff = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let spread = s.iter().cloned().fold(f64::MIN, f64::max)
            - s.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            mean_diff < spread * 0.5,
            "相邻差值 {mean_diff:.3} 相对整体波动 {spread:.3} 过大，不像均值回归"
        );
    }

    #[test]
    fn fatigue_is_monotonic_and_capped() {
        assert_eq!(fatigue_from_km(0.4), 0.0);
        assert!(fatigue_from_km(1.0) > 0.0);
        assert_eq!(fatigue_from_km(5.0), 0.6);
        // 上限 0.6 在 0.9km 处就到（(0.9-0.5)*1.5 = 0.6），
        // 所以要在到达上限之前比单调性
        assert!(fatigue_from_km(0.8) > fatigue_from_km(0.6));
        assert_eq!(fatigue_from_km(0.9), 0.6);
    }
}
