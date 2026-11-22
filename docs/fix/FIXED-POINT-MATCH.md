# 必到点匹配：结果类型、严格半径与证据边界

对应 issue #11 的追加要求。改的是 `api/submit.rs` 的匹配函数，不动校准数值，也不动
默认报文形状（`PACE_FIXED_STRICT` 不开时输出与改前一致）。

## 为什么不再返回 `Vec<usize>`

旧签名 `checkpoint_track_indexes_reserved(...) -> Vec<usize>` 会把没命中的点直接
`filter_map` 掉。调用方只能拿返回长度猜「丢了几个」，配 `fixed[i]` 时还会整体错位：
第 2 个点没命中，第 3 个点就被当成第 2 个报出去。`build_points` 里那条「必到点
『X』10m 内没有可用采样点」的告警因此可能指向错的点。

现在每个点都占一位，命中方式写清楚：

- `MarkMatch::Hit { index, dist_m }` —— 第一轮命中：10 米内、且没被起点/终点/第二点/
  起步灰点占用
- `MarkMatch::Relaxed { index, dist_m }` —— 第二轮兜底：半径内全被占用（或轨迹离点位
  本来就远），放开半径取最近的非占用采样点；`dist_m` 原样带出，偏多远看得见
- `MarkMatch::Missing(reason)` —— 没命中。`EmptyTrack`（轨迹为空）、`StrictRadius`
  （严格模式不放开半径）、`NoCandidate`（非严格模式也找不到）

第一轮仍然取**最近**的采样点，不是「第一个进入半径的点」：采样间距 5~15 米，点位前面
那个点也常在半径内，取第一个会把标记整体往前挪一个采样点（实测点位在第 25 点、被标到
第 24 点）。

## 严格半径

`PACE_FIXED_STRICT=1` 时只跑第一轮，够不着就报 `Missing(StrictRadius)`，不自动放宽
距离。默认关闭，输出与改前逐字段一致。开关已登记进 `pace-forge config`。

这条路径的用途是「宁可少标也不偏标」的核对场景：偏到几十米通常说明点位压根不在轨迹上
（画错围栏 / 人在围栏外），严格模式下会直接报缺失，而不是给一个看起来正常的远处标记。

## 边界测试（纯本地，不含真机数据）

`api/submit.rs` 的 `mark_match_tests`：

1. `reserved_start_point_falls_to_next_sample` —— 起点压在第一个打卡点，0 号被占，命中
   落到 1 号，且是 `Hit` 不是 `Relaxed`
2. `two_checkpoints_can_share_one_sample` —— 采样间距 15 米时两个点位都只够得着同一个
   采样点，结果如实报同一下标
3. `far_point_relaxes_by_default_and_reports_missing_when_strict` —— 点位离线 60 米：
   默认报 `Relaxed` 且距离 ≈60，严格模式报 `Missing(StrictRadius)`
4. `partial_missing_keeps_point_alignment` —— 三个点里第二个缺失，结果仍是 3 项，
   第三项对的是第 3 个点位（旧实现会错位）
5. `empty_inputs_are_explicit` —— 空轨迹、空点位都不 panic，状态显式

## 证据边界：`isPass` 与 `type=2` 是两条独立路径

`fivePointJson` 的 `isPass` 由 `fixed_point_items` 另算：它取「轨迹里**第一个**进入 10 米
半径的采样点」，不看保留点、也不看第二轮的放宽结果。`type=2` 的打卡标记则走本文这套
匹配（避开保留点、必要时放宽半径）。两者输入相同、判据不同，所以会同时出现
「`isPass=true` 但没有 `type=2`」的情形——起点窗口够长时，前十几个采样点都被灰点占着，
标记只能落在更远的采样上。

这能说明的只有：**App 详情页少画一个点**。服务端计分、以及后续审核是否读这套标记，
没有对照样本，不做外推。要断言「标记越界或碰撞不影响计分」，得有服务端打分或审核结果
的实测，目前没有。

## 已知限制

- 两个必到点命中同一个采样下标时不改标记，只打一行 `[points] … 重复下标` 告警：官方
  客户端在「两个点位抢一个采样点」时怎么放标记没有样本依据，乱挪比重叠更危险。
- 严格模式只影响标记与告警，不会让提交失败；要不要在严格模式命不齐时直接拦下，是另一个
  决定（涉及「部分缺失是否算成绩」的协议证据，暂无）。
