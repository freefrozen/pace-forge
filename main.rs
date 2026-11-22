//! 入口：`pace-forge <命令>`
//!
//! 命令：
//! - `web [addr]`      启动 Web 控制台
//! - `dry <距离> <分钟>` 只做流程预演，不发写请求
//! - `stadium <圈数>`   生成操场轨迹并打印参数
//! - `version` / `help`


/// 解析必填或可缺省的数字参数。
///
/// 之前一律 `.and_then(parse().ok()).unwrap_or(默认值)`，于是
/// `run 2 abc` 会被当成 12 分钟**真提交**，`check -1 12` 带着负距离跑完全程，
/// `check 1e9 1e9` 直接 OOM abort。打错字和没写是两回事：
/// 参数存在但解析不出来 → 报错退出；参数不存在 → 用默认值。
fn parse_num(args: &[String], idx: usize, default: f64, name: &str) -> f64 {
    match args.get(idx) {
        None => default,
        Some(raw) => match raw.parse::<f64>() {
            Ok(v) if v.is_finite() => v,
            _ => {
                eprintln!("参数错误：{name} 「{raw}」不是有效数字");
                std::process::exit(2);
            }
        },
    }
}

/// 校验公里/分钟，和库层 `RunParams::validate` 保持同一套上限。
fn checked_km_min(dist_km: f64, dur_min: f64) -> (f64, f64) {
    if !(0.0..=42.0).contains(&dist_km) {
        eprintln!("参数错误：距离要在 0~42 公里之间，收到 {dist_km}");
        std::process::exit(2);
    }
    if !(0.0..=120.0).contains(&dur_min) {
        eprintln!("参数错误：时长要在 0~120 分钟之间，收到 {dur_min}");
        std::process::exit(2);
    }
    (dist_km, dur_min)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("web");

    match cmd {
        "web" => {
            let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9999".into());
            if let Err(e) = pace_forge::web::serve(&addr) {
                eprintln!("启动失败: {e}");
                std::process::exit(1);
            }
        }
        "semester" => show_semester(),
        "elev" => {
            let lat: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(34.04851);
            let lon: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(113.87699);
            if let Some(set) = args.get(3).and_then(|s| s.parse::<f64>().ok()) {
                let ok = pace_forge::api::elev::remember(lat, lon, set);
                println!("{}", if ok { "已写入高程缓存" } else { "数值不合法" });
            }
            match pace_forge::api::elev::lookup(lat, lon) {
                Some(v) => println!("海拔实测 {v:.1} m（{lat:.5}, {lon:.5}）"),
                None => println!("海拔查询失败（{lat:.5}, {lon:.5}）"),
            }
            match pace_forge::api::elev::range_for(lat, lon) {
                Some((lo, hi)) => println!("建议 alt_range = ({lo:.1}, {hi:.1})"),
                None => println!("建议 alt_range = 手填，例如 PACE_ALT_RANGE=\"67,71\""),
            }
        }
        "audit" => do_audit(),
        "tracks" => {
            let n: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(5);
            do_tracks(n);
        }
        "suggest" => do_suggest(),
        "history" => {
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
            do_history(n);
        }
        "cheat" => do_cheat(),
        "logout" => do_logout(),
        "relogin" => do_relogin(),
        "check" => {
            let dist_km = parse_num(&args, 1, 2.0, "距离(km)");
            let dur_min = parse_num(&args, 2, 12.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            local_check(dist_km, dur_min);
        }
        "run" => {
            let dist_km = parse_num(&args, 1, 2.0, "距离(km)");
            let dur_min = parse_num(&args, 2, 12.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            let yes = args.iter().any(|a| a == "-y" || a == "--yes");
            let mode = args
                .iter()
                .position(|a| a == "--mode")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok());
            do_run(dist_km, dur_min, yes, mode);
        }
        "begin" => {
            // begin <km> <min> [--auto]：现在就是「开始跑」的时刻，
            // 记下起点，等够时长再提交（真人就是先按开始、22 分钟后提交）。
            let dist_km = parse_num(&args, 1, 2.4, "距离(km)");
            let dur_min = parse_num(&args, 2, 22.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            let auto = args.iter().any(|a| a == "--auto");
            let mode = args
                .iter()
                .position(|a| a == "--mode")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok());
            do_begin(dist_km, dur_min, auto, mode);
        }
        "finish" => do_finish(),
        "dry" => {
            let dist_km = parse_num(&args, 1, 2.0, "距离(km)");
            let dur_min = parse_num(&args, 2, 12.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            dry_run(dist_km, dur_min);
        }
        "at" => {
            // at <HH:MM> <km> <min> [--days N]
            let hhmm = args.get(1).cloned().unwrap_or_default();
            let dist_km = parse_num(&args, 2, 2.0, "距离(km)");
            let dur_min = parse_num(&args, 3, 12.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            let days: i64 = args
                .iter()
                .position(|a| a == "--days")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let yes = args.iter().any(|a| a == "-y" || a == "--yes");
            do_at(&hhmm, days, dist_km, dur_min, yes);
        }
        "ago" => {
            // ago <分钟> <km> <min>
            let mins: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let dist_km = parse_num(&args, 2, 2.0, "距离(km)");
            let dur_min = parse_num(&args, 3, 12.0, "时长(min)");
            let (dist_km, dur_min) = checked_km_min(dist_km, dur_min);
            let yes = args.iter().any(|a| a == "-y" || a == "--yes");
            do_ago(mins, dist_km, dur_min, yes);
        }
        "rand" => {
            // rand <km> <min> [--days N]
            let dist_km: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2.0);
            let dur_min: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(12.0);
            let days: i64 = args
                .iter()
                .position(|a| a == "--days")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let yes = args.iter().any(|a| a == "-y" || a == "--yes");
            do_rand(days, dist_km, dur_min, yes);
        }
        "stadium" => {
            let laps: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
            let lane: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            stadium_probe(laps, lane);
        }
        "version" | "-V" | "--version" => {
            println!("pace-forge {}", pace_forge::VERSION);
        }
        "help" | "-h" | "--help" => {
            println!("pace-forge {} — 运动世界校园协议工具\n", pace_forge::VERSION);
            println!("用法:");
            println!("  pace-forge web [addr]          启动 Web 控制台（默认 127.0.0.1:9999）");
            println!("  pace-forge semester            查学期进度（记分跑）");
            println!("  pace-forge run [km] [min] [-y] [--mode N]  刷跑并提交（-y 跳过确认）");
            println!("                                 N: 1=校园跑/记分(默认) 2=户外跑 3=室外锻炼");
            println!("  pace-forge check [km] [min]    离线体检轨迹（不联网、不登录）");
            println!("  pace-forge dry [km] [min]      只预演，不提交（要登录）");
            println!("  pace-forge begin [km] [min] [--auto]  现在开始跑：记下起点，到点再提交");
            println!("  pace-forge finish              到点执行：传明细 + 提交（配合 begin）");
            println!("  pace-forge at <HH:MM> [km] [min] [--days N]  补跑：指定当天/往前的时刻");
            println!("  pace-forge ago <分钟> [km] [min]            补跑：N 分钟前");
            println!("  pace-forge rand [km] [min] [--days N]       补跑：当天 7-20 点随机时刻");
            println!("  pace-forge history [N]        拉最近 N 条记录明细并落盘（只读）");
            println!("  pace-forge audit              账号风控体检（连续天数/时刻/里程离散度）");
            println!("  pace-forge suggest            今天该不该刷、刷多少（只读）");
            println!("  pace-forge tracks [N]         轨迹重合度体检（拉 OBS 明细比对，默认 5 条）");
            println!("  pace-forge stadium [圈] [道次] 操场轨迹参数");
            println!("  pace-forge version             版本号");
            println!();
            println!("规则：开始 + 用时 + 5s 余量 ≤ 现在，最多往前补 3 天。");
            println!("run/dry/at/ago/rand 需要已保存的登录会话（$HOME/.pace-forge/session.json）。");
        }
        other => {
            eprintln!("未知命令: {other}\n用 `pace-forge help` 查看用法");
            std::process::exit(2);
        }
    }
}

fn dry_run(dist_km: f64, dur_min: f64) {
    use pace_forge::api::{flow, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先通过 Web 控制台登录一次（会话存在 $HOME/.pace-forge/session.json）");
        std::process::exit(1);
    }
    let client = pace_forge::api::client::Client::new(pace_forge::api::client::DeviceProfile::default())
        .with_credentials(pace_forge::api::client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        });

    // 中心点必须从真实围栏反推。
    //
    // CLI 原来走 RunParams::default()，center 是 (0.0, 0.0)——赤道的几内亚湾。
    // 围栏选择按「最近」兜底，会选中 2677 公里外的学校，再交给
    // route_through_in_order 就算出 7640 公里、152 万个点的鬼轨迹。
    // fetch_fences 保证返回的每个围栏都已成环（点数 >= 3），
    // 所以这里不需要再判断 ring 是否为空。
    let (clat, clng) = match pace_forge::api::fence::fetch_fences(&client) {
        Ok(fs) => {
            let ring = &fs[0].ring;
            let n = ring.len() as f64;
            let s = ring
                .iter()
                .fold((0.0, 0.0), |a, q| (a.0 + q.0, a.1 + q.1));
            (s.0 / n, s.1 / n)
        }
        Err(e) => {
            eprintln!("拿不到围栏中心点，无法规划轨迹: {e}");
            std::process::exit(1);
        }
    };
    println!("围栏中心: ({clat:.5}, {clng:.5})");

    let p = flow::RunParams {
        center_lat: clat,
        center_lng: clng,
        distance_m: dist_km * 1000.0,
        duration_sec: (dur_min * 60.0).round() as i64,
        ..Default::default()
    };

    println!(
        "预演：{:.1} km / {:.0} 分钟（不发任何写请求）\n",
        dist_km, dur_min
    );
    let mut log = |s: &str| println!("{s}");
    match flow::dry_run(&client, &p, &mut log) {
        Ok(o) => {
            println!("\n=== 完成 ===");
            println!("  规划点数    {}", o.track_points);
            println!("  实际里程    {:.1} m", o.actual_length_m);
            println!("  总步数      {}", o.total_steps);
            println!("  平均步频    {:.1} 步/分", o.avg_cadence);
            println!("  自检        {}", o.report.summary());
            println!("  已提交      {}", if o.submitted { "是" } else { "否（预演）" });
        }
        Err(e) => {
            eprintln!("\n预演失败: {e}");
            std::process::exit(1);
        }
    }
}

/// 账号风控体检（只读）。
fn do_audit() {
    use pace_forge::api::{client, records, risk, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先在 http://127.0.0.1:9999 登录一次");
        std::process::exit(1);
    }
    let c = client::Client::new(client::DeviceProfile::default()).with_credentials(
        client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        },
    );
    let list = match records::list_records(&c, 1, 40, 0) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("拉取记录失败: {e}");
            std::process::exit(1);
        }
    };
    let runs = risk::parse_runs(&list);
    if runs.is_empty() {
        println!("没有历史记录，无法体检");
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let rep = risk::assess(&runs, now);

    println!("账号风控体检（最近 {} 条记录）", runs.len());
    println!("  风险分      {:.0} / 100  → {}", rep.score, rep.grade());
    println!("  连续天数    {} 天", rep.streak_days);
    println!("  最近 7 天   {} 天有记录", rep.days_last_7);
    println!(
        "  时刻集中    {} 点占 {:.0}%",
        rep.top_hour,
        rep.hour_concentration * 100.0
    );
    println!("  里程离散    变异系数 {:.1}%", rep.dist_cv * 100.0);
    println!("  配速离散    变异系数 {:.1}%", rep.pace_cv * 100.0);
    println!("  同日多条    {} 天", rep.same_day_days);
    println!("  日均里程    {:.2} km", rep.mean_km);
    if rep.flags.is_empty() {
        println!("\n没有发现排班式特征。");
    } else {
        println!("\n扣分项：");
        for (pts, msg) in &rep.flags {
            println!("  -{pts:<3} {msg}");
        }
        println!("\n注意：这只是**本地统计**，服务端的跑批模型看不到。");
        println!("      真正的「有没有被标记」看 `pace-forge cheat`（服务端 cheatlist 自查）。");
    }
}

/// 轨迹重合度体检：把最近 N 条记录的轨迹从 OBS 拉下来两两比对。
///
/// 存在意义：全校都每天跑 3 公里时，「里程/时长/连续天数」都不是区分特征，
/// 真正的区分点是**这些记录是不是同一根线**。脚本产出的轨迹由同一套算法与
/// 同一批点位决定，连日跑几乎重合；真人同一条路线每次也会差 8~20 米。
fn do_tracks(n: usize) {
    use pace_forge::api::{client, obs, records, risk, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先在 http://127.0.0.1:9999 登录一次");
        std::process::exit(1);
    }
    let c = client::Client::new(client::DeviceProfile::default()).with_credentials(
        client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        },
    );
    let list = match records::list_records(&c, 1, n as u32, 0) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("拉取记录失败: {e}");
            std::process::exit(1);
        }
    };
    let rows = list
        .get("records")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    println!("最近 {} 条记录的轨迹重合度（只读）\n", rows.len());
    let mut got: Vec<(i64, Vec<(f64, f64)>)> = Vec::new();
    for r in rows.iter() {
        let rrid = r.get("rrid").and_then(|v| v.as_i64()).unwrap_or(0);
        let uuid = r.get("uuid").and_then(|v| v.as_str()).unwrap_or_default();
        let start_ms = r.get("startTime").and_then(|v| v.as_i64()).unwrap_or(0);
        if uuid.is_empty() || start_ms == 0 {
            continue;
        }
        let keys = [
            obs::detail_key(start_ms, uuid),
            obs::detail_key_utc(start_ms, uuid),
            format!("run_data/{}/{rrid}.json", rrid / 1_000_000),
        ];
        let mut pts = Vec::new();
        let mut last_err = String::new();
        for k in keys.iter() {
            match obs::fetch_object(&c, k) {
                Ok(v) => {
                    pts = risk::extract_points(&v);
                    if pts.len() >= 4 {
                        break;
                    } else {
                        last_err = format!("{k}: JSON 里没有点位（{v:.80}）");
                    }
                }
                Err(e) => last_err = format!("{k}: {e}"),
            }
        }
        if pts.len() >= 4 {
            println!("  {rrid}  明细 {} 点", pts.len());
            got.push((rrid, pts));
        } else {
            println!("  {rrid}  明细缺失：{last_err}");
        }
    }

    if got.len() < 2 {
        println!("\n可用的轨迹不足两条，无法比对。");
        return;
    }
    println!("\n两两重合度：");
    let mut worst = (0.0f64, 0i64, 0i64);
    for i in 0..got.len() {
        for j in (i + 1)..got.len() {
            let s = risk::similarity(&got[i].1, &got[j].1);
            println!("  {} vs {}  {:>5.1}%", got[i].0, got[j].0, s * 100.0);
            if s > worst.0 {
                worst = (s, got[i].0, got[j].0);
            }
        }
    }
    println!(
        "\n最高重合度 {:.1}%（{} vs {}）",
        worst.0 * 100.0,
        worst.1,
        worst.2
    );
    if worst.0 >= 0.85 {
        println!("⚠ 超过 85%：这几条几乎是同一条线，是脚本最容易被认出来的形态。");
        println!("  建议换路线（改起点/改绕行）、或隔几天再刷。");
    } else if worst.0 >= 0.7 {
        println!("· 70~85%：偏低风险，但同一时段连着跑也会被看出来。");
    } else {
        println!("✓ 低于 70%，轨迹形态分散。");
    }
}

/// 今天的刷跑建议（只读）。
fn do_suggest() {
    use pace_forge::api::{client, records, risk, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先在 http://127.0.0.1:9999 登录一次");
        std::process::exit(1);
    }
    let c = client::Client::new(client::DeviceProfile::default()).with_credentials(
        client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        },
    );
    let list = match records::list_records(&c, 1, 40, 0) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("拉取记录失败: {e}");
            std::process::exit(1);
        }
    };
    let runs = risk::parse_runs(&list);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let seed = (now / 1000) as u64;
    let s = risk::suggest(&runs, now, seed);

    if !s.allowed {
        println!("今天不建议刷：{}", s.reason);
        return;
    }
    println!("{}", s.reason);
    let pace_sec = s.minutes as f64 * 60.0 / s.km;
    println!(
        "\n建议参数：{:.1} km / {} 分钟（平均配速约 {}'{:02}\" 每公里）",
        s.km,
        s.minutes,
        (pace_sec / 60.0).floor() as i64,
        (pace_sec % 60.0) as i64
    );
    println!("建议时段：{} 点前后", s.hour);
    println!("\n直接可用：");
    println!(
        "  PACE_CAD_INTERCEPT=58.7 PACE_CAD_SLOPE=10.8 PACE_STRIDE_MAX=3.4 \\"
    );
    println!("    ./target/release/pace-forge run {} {} -y", s.km, s.minutes);
}

/// 拉最近 N 条记录（只读），落盘成 JSON 供校准用。
///
/// 为什么要落盘：真正的校准输入是**逐点明细**（`allLocJson` / OBS 对象），
/// 光看列表里的平均步频无从建模。把原始响应原样存下来，后面写校准脚本时
/// 不用再登录、不用再联网。
fn do_history(n: u32) {
    use pace_forge::api::{client, records, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先通过 Web 控制台登录一次（会话存在 $HOME/.pace-forge/session.json）");
        std::process::exit(1);
    }
    let c = client::Client::new(client::DeviceProfile::default()).with_credentials(
        client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        },
    );

    let list = match records::list_records(&c, 1, n, 0) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("拉取记录列表失败: {e}");
            std::process::exit(1);
        }
    };
    let rows = list
        .get("records")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    println!(
        "记录共 {} 条，本页取到 {} 条（只读，不发任何写请求）\n",
        list.get("total").and_then(|v| v.as_i64()).unwrap_or(-1),
        rows.len()
    );
    println!(
        "{:>14} {:<19} {:>8} {:>9} {:>8} {:>7} {:<6}",
        "rrid", "开始时间", "里程(m)", "时长(s)", "配速", "步频", "达标"
    );
    for r in rows.iter() {
        let rrid = r.get("rrid").and_then(|v| v.as_i64()).unwrap_or(0);
        let start = r
            .get("startTime")
            .or_else(|| r.get("start_time"))
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let dis = r.get("totalDistance").and_then(|v| v.as_i64()).unwrap_or(0);
        let sec = r.get("totalTime").and_then(|v| v.as_i64()).unwrap_or(0);
        let pace = if dis > 0 && sec > 0 {
            let s = sec as f64 / (dis as f64 / 1000.0);
            format!("{:>2}'{:02}\"/km", (s / 60.0) as i64, (s % 60.0) as i64)
        } else {
            "-".to_string()
        };
        let cad = r.get("avgStepFreq").and_then(|v| v.as_i64()).unwrap_or(0);
        let ok = r
            .get("completed")
            .and_then(|v| v.as_bool())
            .map(|b| if b { "是" } else { "否" })
            .unwrap_or("-");
        println!("{rrid:>14} {start:<19} {dis:>8} {sec:>9} {pace:>8} {cad:>7} {ok:<6}");
    }

    // 明细：第一条记录拉详情，看有没有点位可用作校准样本。
    if let Some(first) = rows.first() {
        let rrid = first.get("rrid").and_then(|v| v.as_i64()).unwrap_or(0);
        match records::get_record(&c, rrid) {
            Ok(d) => {
                let keys: Vec<String> = d
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                let has_points = d
                    .get("allLocJson")
                    .and_then(|v| v.as_str())
                    .map(|s| s.len())
                    .unwrap_or(0);
                println!("\n详情 rrid={rrid}：{} 个字段，allLocJson 长度 {has_points}", keys.len());
                let dir = pace_forge::api::session::data_dir();
                let _ = std::fs::create_dir_all(&dir);
                let p1 = dir.join("history-list.json");
                let p2 = dir.join("history-detail.json");
                let _ = std::fs::write(&p1, serde_json::to_string_pretty(&list).unwrap_or_default());
                let _ = std::fs::write(&p2, serde_json::to_string_pretty(&d).unwrap_or_default());
                println!("已落盘：{}\n         {}", p1.display(), p2.display());
            }
            Err(e) => eprintln!("拉详情失败: {e}"),
        }
    }
}

/// 完全离线的轨迹体检：不登录、不联网，只看本地合成的数据像不像真跑。
fn local_check(dist_km: f64, dur_min: f64) {
    use pace_forge::track::{
        buckets::{make_jitter, speed_buckets},
        detect::{inspect, PointRule},
        simulate, KinParams, SimParams, StadiumSpec,
    };

    let total_m = dist_km * 1000.0;
    let total_sec = (dur_min * 60.0).round() as i64;
    let start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    println!("离线体检：{dist_km:.1} km / {dur_min:.0} 分钟");
    println!("（不登录、不发请求，只验证本地合成数据）\n");

    // 用操场轨迹，几何确定，便于看点位间距
    let spec = StadiumSpec::new(30.5, 104.0).with_lane(1);
    let lap = pace_forge::track::lap_length(1);
    let laps = (total_m / lap).max(0.5);
    let xy = pace_forge::track::stadium_track(&spec, laps, 5.0);
    println!("  轨迹点      {}", xy.len());
    println!("  绕圈        {laps:.2} 圈（单圈 {lap:.1} m）");

    // 上报采样：等时间 5 秒（与 `flow::dry_run` 同一条路径）
    let times = pace_forge::track::sample_times(
        total_sec,
        start_ms,
        pace_forge::track::SAMPLE_INTERVAL_SEC,
    );
    let track_points = pace_forge::track::sample_by_time(
        &xy, &times, start_ms as u64, &KinParams::default(), 1.0, 8.0, &[], &[],
    );
    let real_len = pace_forge::track::geo::path_length(
        &track_points.iter().map(|q| (q.lat, q.lon)).collect::<Vec<_>>(),
    ).round() as i64;
    let cad = pace_forge::track::cadence_stats(&track_points);
    println!(
        "  上报节奏    {} 点 / {:.2}s {:.1}m 一点（真机 {:.2}s {:.1}m）",
        cad.points, cad.mean_dt_sec, cad.mean_dd_m,
        pace_forge::track::REAL_MEAN_INTERVAL_SEC, 14.9,
    );
    let pts3: Vec<(f64, f64, i64)> = track_points.iter()
        .map(|q| (q.lat, q.lon, q.time_ms)).collect();

    // 分桶
    let sb = speed_buckets(&pts3, start_ms, total_sec);
    // 这里原来还造了一份 step_buckets(..., 180, &jit) 赋给 stb 却从没用过
    // （clippy 一直报 unused variable）——分桶由 simulate() 产出。
    let _ = make_jitter(sb.len().max(1), 1.0, start_ms as u64);

    // 合成
    let sp = SimParams {
        total_distance_m: real_len as f64,
        total_sec,
        start_ms,
        seed: start_ms as u64,
        ..Default::default()
    };
    let sim = match simulate(&pts3, &sp) {
        Ok(s) => s,
        Err(e) => { eprintln!("合成失败: {e}"); return; }
    };

    println!("  实际里程    {:.1} m", real_len);
    println!("  总步数      {}", sim.total_steps);
    println!("  平均步频    {:.1} 步/分", sim.avg_cadence);

    // 反检测体检
    let rep = inspect(
        &pts3.iter().map(|(a, b, _)| (*a, *b)).collect::<Vec<_>>(),
        &sb,
        PointRule::default(),
        start_ms + pace_forge::track::startup_sec(total_sec) * 1000,
    );
    println!("\n=== 点位体检（模拟 7.4.01 本地规则）===");
    println!("  {}", rep.summary());
    for line in rep.lines().iter().take(8) {
        println!("  · {line}");
    }
    println!(
        "\n  判定: {}",
        if rep.blocked() {
            "✗ 会被判异常"
        } else {
            "✓ 通过"
        }
    );

    // 分桶统计
    let caps: Vec<f64> = sb.iter().map(|b| b.distance).collect();
    if !caps.is_empty() {
        let mx = caps.iter().cloned().fold(f64::MIN, f64::max);
        let mn = caps.iter().cloned().fold(f64::MAX, f64::min);
        let avg = caps.iter().sum::<f64>() / caps.len() as f64;
        println!("  分桶        {} 个，{:.1}~{:.1} m（均值 {:.1}）", caps.len(), mn, mx, avg);
    }
}

/// 用本地保存的密码重新登录，刷新 token。
///
/// 密码在 accounts.json 里是加密存的，这里解出来直接用，不用重新输入。
fn do_relogin() {
    use pace_forge::api::{accounts, client, login as login_api, session};

    let mut book = accounts::load();
    let acc = match book.accounts.first().cloned() {
        Some(a) => a,
        None => {
            eprintln!("本地没有账号记录");
            std::process::exit(1);
        }
    };
    if acc.password_enc.is_empty() {
        eprintln!("账号 {} 没存密码，需要重新输入", acc.username);
        std::process::exit(1);
    }
    let password = match accounts::open_password(&acc.password_enc) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("密码解密失败: {e}");
            std::process::exit(1);
        }
    };

    let mut c = client::Client::new(client::DeviceProfile::default());
    match login_api::login(&mut c, &acc.username, &password) {
        Ok(login_api::LoginOutcome::Ok(prof)) => {
            let creds = &c.creds;
            let sess = session::Session {
                uid: creds.uid.clone().unwrap_or_default(),
                token: creds.token.clone().unwrap_or_default(),
                unid: creds.unid.clone().unwrap_or_default(),
                username: acc.username.clone(),
                name: prof.name.clone(),
                campus_name: prof.campus_name.clone(),
                depart: prof.depart.clone(),
                grade_class: prof.grade_class.clone(),
                device_id: String::new(),
            };
            if let Err(e) = session::save_session(&sess) {
                eprintln!("session 写入失败: {e}");
            }
            if let Some(a) = book.get_mut(&acc.id) {
                a.uid = sess.uid.clone();
                a.token = sess.token.clone();
                a.unid = sess.unid.clone();
            }
            let _ = accounts::save(&book);
            println!("✅ 重新登录成功：{} ({})", sess.name, acc.username);
        }
        Ok(login_api::LoginOutcome::NeedCaptcha { message, .. }) => {
            eprintln!("需要过滑动验证码（{message}），请在网页里登录一次");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("登录失败: {e}");
            std::process::exit(1);
        }
    }
}

/// 通知服务端注销当前账号，成功后清掉本地 token。
///
/// 只删本地 session 不等于退出登录——服务端的 token 仍然有效，
/// 换台设备照样能用。这里真的打一次 logout 接口。
fn do_logout() {
    let book = pace_forge::api::accounts::load();
    let acc = match book.accounts.first() {
        Some(a) => a.clone(),
        None => {
            eprintln!("本地没有账号记录，无需注销");
            return;
        }
    };
    let mut client =
        pace_forge::api::client::Client::new(pace_forge::api::client::DeviceProfile::default())
            .with_credentials(pace_forge::api::client::Credentials {
                token: Some(acc.token.clone()),
                uid: Some(acc.uid.clone()),
                unid: Some(acc.unid.clone()),
            });

    match pace_forge::api::login::logout(&mut client) {
        Ok(true) => println!("✅ 服务端已注销 {}", acc.username),
        Ok(false) => println!("服务端没有返回明确的注销结果，可能是 token 已失效"),
        Err(e) => {
            eprintln!("注销请求失败: {e}");
            std::process::exit(1);
        }
    }

    // 本地也要清，否则下次还会带着这枚已经作废的 token 去请求
    if let Err(e) = pace_forge::api::accounts::clear_tokens() {
        eprintln!("本地 token 清理失败: {e}");
    } else {
        println!("本地 token 已清除");
    }
}

/// 违规名单自查。提交成功不代表没被标记——跑批判定是事后做的。
fn do_cheat() {
    let sess = pace_forge::api::session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先执行 `pace-forge web` 在网页里登录一次");
        std::process::exit(1);
    }
    let client = pace_forge::api::client::Client::new(
        pace_forge::api::client::DeviceProfile::default(),
    )
    .with_credentials(pace_forge::api::client::Credentials {
        uid: Some(sess.uid.to_string()),
        token: Some(sess.token.clone()),
        unid: Some(sess.unid.clone()),
    });
    let mut last = String::new();
    match pace_forge::api::cheat::query(&client, 1, &mut |l| {
        last = l.to_string();
    }) {
        Ok(report) => {
            println!("{last}");
            // 「未知」不能报成「清白」——退出码区分开，脚本里才判得对。
            use pace_forge::api::cheat::Status;
            match report.status() {
                Status::Flagged => std::process::exit(2),
                Status::Unknown | Status::Stale => std::process::exit(3),
                Status::NoRecord => {}
            }
        }
        Err(e) => {
            eprintln!("查询失败: {e}");
            std::process::exit(1);
        }
    }
}

fn show_semester() {
    use pace_forge::api::{semester, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先执行 `pace-forge web` 在网页里登录一次");
        std::process::exit(1);
    }
    let client = pace_forge::api::client::Client::new(pace_forge::api::client::DeviceProfile::default())
        .with_credentials(pace_forge::api::client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        });

    match semester::fetch_summary(&client) {
        Ok(sm) => {
            println!("{}", sm.summary());
            println!("  有效比例 {:.1}%", sm.valid_ratio() * 100.0);
        }
        Err(e) => eprintln!("查学期汇总失败: {e}"),
    }

    match semester::fetch_goal(&client, 1) {
        Ok(g) => {
            if g.target_dis > 0.0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                println!(
                    "学期目标 {:.1}km / {} 次，还剩 {} 天",
                    g.target_dis / 1000.0,
                    g.target_count,
                    g.days_left(now)
                );
                if !g.in_term(now) {
                    println!("⚠ 当前不在学期内");
                }
            } else {
                println!("· 未取到学期目标");
            }
        }
        Err(e) => eprintln!("查学期目标失败: {e}"),
    }
}

/// 一次「在跑」的状态：起点时刻 + 参数。
#[derive(serde::Serialize, serde::Deserialize)]
struct PendingRun {
    start_ms: i64,
    stop_ms: i64,
    km: f64,
    minutes: i64,
}

fn pending_path() -> std::path::PathBuf {
    pace_forge::api::session::data_dir().join("pending-run.json")
}

/// 现在就是「开始跑」这一刻：记下起点，等到点再提交。
///
/// 为什么要有这个命令：服务端按 `startTime`/`stopTime` 算这条记录的时间窗，
/// 而真人的时间线是「点开始 → 跑 22 分钟 → 点结束 → 提交」。我们如果登录完
/// 直接提交，日志里就是「登录后 40 秒交了一条 22 分钟的记录」——这是风控
/// 一眼能看出的矛盾，而且不需要任何深度分析。
///
/// 用法：
/// ```text
/// pace-forge begin 2.4 22            # 记下起点，手动 finish
/// pace-forge begin 2.4 22 --auto     # 挂后台，到点自动提交
/// ```
fn do_begin(dist_km: f64, dur_min: f64, auto: bool, mode: Option<i64>) {
    use pace_forge::api::session;

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先在 http://127.0.0.1:9999 登录一次");
        std::process::exit(1);
    }
    let now = pace_forge::crypto::envelope::now_ms() as i64;
    let dur_s = (dur_min * 60.0).round() as i64;
    let p = PendingRun {
        start_ms: now,
        stop_ms: now + dur_s * 1000,
        km: dist_km,
        minutes: dur_min.round() as i64,
    };
    let dir = session::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(&p) {
        let _ = std::fs::write(pending_path(), s);
    }
    println!(
        "开始记录：{:.1} km / {} 分钟，起点 {}，结束 {}",
        dist_km,
        p.minutes,
        fmt_ms(p.start_ms),
        fmt_ms(p.stop_ms)
    );
    println!("  到点执行： pace-forge finish");
    if !auto {
        return;
    }
    // --auto：睡到结束时刻之后 3 秒再提交（真人是点结束后几秒才提交的）。
    let wait_ms = (p.stop_ms + 3_000 - pace_forge::crypto::envelope::now_ms() as i64).max(0);
    println!("  自动模式：等待 {} 分 {} 秒后提交…", wait_ms / 60_000, (wait_ms / 1000) % 60);
    std::thread::sleep(std::time::Duration::from_millis(wait_ms as u64));
    do_finish_with(mode);
}

fn do_finish() {
    do_finish_with(None);
}

fn do_finish_with(mode: Option<i64>) {
    let raw = match std::fs::read_to_string(pending_path()) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("没有进行中的记录：先跑 `pace-forge begin <km> <min>`");
            std::process::exit(1);
        }
    };
    let p: PendingRun = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("进行中的记录损坏（{e}）：删掉 {} 重来", pending_path().display());
            std::process::exit(1);
        }
    };
    let now = pace_forge::crypto::envelope::now_ms() as i64;
    if now + 2_000 < p.stop_ms {
        eprintln!(
            "还没跑完：还要等 {} 分 {} 秒（到点再执行 finish）",
            (p.stop_ms - now) / 60_000,
            ((p.stop_ms - now) / 1000) % 60
        );
        std::process::exit(1);
    }
    let _ = std::fs::remove_file(pending_path());
    run_with_start(p.km, p.minutes as f64, Some(p.start_ms), true, mode);
}

fn fmt_ms(ms: i64) -> String {
    // 只用于打印，按 UTC+8 粗算
    let secs = ms / 1000 + 8 * 3600;
    let (d, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let _ = d;
    format!("{h:02}:{m:02}:{s:02}")
}

fn do_run(dist_km: f64, dur_min: f64, yes: bool, mode: Option<i64>) {
    run_with_start(dist_km, dur_min, None, yes, mode);
}

/// 通用提交：`start_at` 为 `None` 表示补一条**刚跑完**的记录
///（结束时刻 = 服务器现在，起跑时刻倒推一整个时长）。
fn run_with_start(
    dist_km: f64,
    dur_min: f64,
    start_at: Option<i64>,
    yes: bool,
    mode: Option<i64>,
) {
    use pace_forge::api::{flow, session};

    let sess = session::load_session();
    if !sess.is_valid() {
        eprintln!("未登录：先执行 `pace-forge web` 在网页里登录一次");
        std::process::exit(1);
    }
    let client = pace_forge::api::client::Client::new(pace_forge::api::client::DeviceProfile::default())
        .with_credentials(pace_forge::api::client::Credentials {
            uid: Some(sess.uid.to_string()),
            token: Some(sess.token.clone()),
            unid: Some(sess.unid.clone()),
        });

    // 中心点同样必须从真实围栏反推，不能用默认的 (0,0)。
    let (clat, clng) = match pace_forge::api::fence::fetch_fences(&client) {
        Ok(fs) => {
            let ring = &fs[0].ring;
            let n = ring.len() as f64;
            let t = ring
                .iter()
                .fold((0.0, 0.0), |a, q| (a.0 + q.0, a.1 + q.1));
            (t.0 / n, t.1 / n)
        }
        Err(e) => {
            eprintln!("拿不到围栏中心点，无法规划轨迹: {e}");
            std::process::exit(1);
        }
    };

    let p = flow::RunParams {
        center_lat: clat,
        center_lng: clng,
        distance_m: dist_km * 1000.0,
        duration_sec: (dur_min * 60.0).round() as i64,
        start_at_ms: start_at,
        custom_path: None,
        // 默认 **1 = 校园跑（记分跑）**，不是 2。
        //
        // 原来默认 2（户外跑），理由是「记分跑有时段窗口，容易被 11016 挡掉」。
        // 但这个默认会让**成绩根本不计入校园跑**：2026-11-18 实测，用默认值
        // 提交的那条落到户外跑里，App 的「成绩/次数」看不到它（学期有效里程
        // 只涨了 0.87km，总里程涨 2.41km）。是否有时段限制应该由 policy 的
        // `validTime` 决定（许昌学院实测 validTime=0，全天可跑），而不是靠
        // 默认成「不计分」的模式来回避。
        //
        // 覆盖方式：`--mode 2`（户外跑）或 `PACE_RUN_MODE=2`。
        run_mode: mode
            .or_else(|| {
                std::env::var("PACE_RUN_MODE")
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(1),
        sample_step_m: std::env::var("PACE_STEP_M")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5.0),
        ..Default::default()
    };

    println!("刷跑：{:.1} km / {:.0} 分钟\n", dist_km, dur_min);
    let mut log = |s: &str| println!("{s}");
    let mut confirm = |o: &flow::RunOutcome| -> bool {
        println!();
        println!("=== 待提交 ===");
        println!("  轨迹点   {}", o.track.len());
        println!("  里程     {:.1} m", o.actual_length_m);
        println!("  步数     {}", o.total_steps);
        println!("  步频     {:.1} 步/分", o.avg_cadence);
        println!("  自检     {}", o.report.summary());
        if yes {
            println!("  （-y 已跳过确认）");
            return true;
        }
        print!("\n确认提交？[y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        matches!(line.trim(), "y" | "Y" | "yes")
    };

    match flow::run_full(&client, &p, &mut log, &mut confirm) {
        Ok(o) => {
            if o.submitted {
                println!("\n✅ 提交成功，记录 id={:?}", o.record_id);
                println!("   {} 秒内不要再跑", flow::SILENCE_SEC);
            } else {
                println!("\n已取消");
            }
        }
        Err(e) => {
            eprintln!("\n失败: {e}");
            std::process::exit(1);
        }
    }
}

fn stadium_probe(laps: f64, lane: u8) {
    use pace_forge::track::{arc_and_curvature, lap_length, path_length, stadium_track, StadiumSpec};

    let spec = StadiumSpec::new(30.5, 104.0).with_lane(lane);
    let pts = stadium_track(&spec, laps, 5.0);
    let len = path_length(&pts);
    let (_, k) = arc_and_curvature(&pts);
    let max_k = k.iter().cloned().fold(0.0f64, f64::max);

    println!("操场第 {lane} 道");
    println!("  单圈      {:.2} m", lap_length(lane));
    println!("  {laps:.1} 圈      {:.1} m", len);
    println!("  采样点    {}", pts.len());
    println!("  最大曲率  {max_k:.5} /m（理论 {:.5}）", 1.0 / (36.5 + 0.3 + 1.22 * (lane as f64 - 1.0)));
    println!();
    println!("建议：{:.0}m 跑 {:.0} 秒（配速 {:.2} 分/km）", len, (len / 2.78).round(), 1000.0 / 2.78 / 60.0);
}


// ---------------------------------------------------------------------------
// 补跑：指定时间
// ---------------------------------------------------------------------------

fn show_plan(st: pace_forge::api::start_time::StartTime, dur_min: f64) {
    let dur_s = (dur_min * 60.0).round() as i64;
    println!("开始 {}", pace_forge::api::start_time::format_ms(
        st.ms, pace_forge::api::start_time::default_tz_offset_min()));
    println!("结束 {}", pace_forge::api::start_time::format_ms(
        st.stop_ms(dur_s), pace_forge::api::start_time::default_tz_offset_min()));
    if st.clamped {
        println!("注意：原时刻会让结束时间落在未来，已自动提前");
    }
}

fn do_at(hhmm: &str, days: i64, dist_km: f64, dur_min: f64, yes: bool) {
    use pace_forge::api::start_time as st_mod;
    let (h, m) = match st_mod::parse_hhmm(hhmm) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let now = pace_forge::crypto::envelope::now_ms() as i64;
    let dur_s = (dur_min * 60.0).round() as i64;
    let st = match st_mod::specified(now, days, h, m, dur_s) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    show_plan(st, dur_min);
    run_with_start(dist_km, dur_min, Some(st.ms), yes, None);
}

fn do_ago(mins: i64, dist_km: f64, dur_min: f64, yes: bool) {
    use pace_forge::api::start_time as st_mod;
    let now = pace_forge::crypto::envelope::now_ms() as i64;
    let dur_s = (dur_min * 60.0).round() as i64;
    let st = match st_mod::ago(now, mins, dur_s) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    show_plan(st, dur_min);
    run_with_start(dist_km, dur_min, Some(st.ms), yes, None);
}

fn do_rand(days: i64, dist_km: f64, dur_min: f64, yes: bool) {
    use pace_forge::api::start_time as st_mod;
    use rand::Rng;
    let now = pace_forge::crypto::envelope::now_ms() as i64;
    let dur_s = (dur_min * 60.0).round() as i64;
    let r: f64 = rand::thread_rng().gen();
    let st = match st_mod::random_in_day(now, days, dur_s, r) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    show_plan(st, dur_min);
    run_with_start(dist_km, dur_min, Some(st.ms), yes, None);
}
