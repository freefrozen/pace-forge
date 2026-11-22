# OBS 明细对象的形状与编码

上传分两步：`POST /api/obs/temporary/url` 换签名 URL，再 `PUT` 对象本体。这份笔记只
记「对象长什么样、key 怎么拼」，不记签名流程。

## 对象路径

两条路径都发，服务端认哪条就用哪条：

- `run_data/{YYYYMMDDHH}/{uuid}.json` —— 详情页展示用，也是实测命中过的那条
- `runData/{uid}/{sportType}/{startTime}_{uuid}.txt` —— 另一种说法，实测同样存在

小时目录里的 `{YYYYMMDDHH}` 取**记录开始时间**，不是结束时间也不是上传时间。代码里
`detail_key(start_time_ms, …)`（`api/flow.rs` 的 OBS 段、`api/submit.rs` 的 key 生成），
值就是提交窗口的起点。

因为服务端拼 key 时用哪个时区没有确定证据（服务器按 `ZoneId.systemDefault()` 还是
`ZoneOffset.UTC` 都能解释），这里 `run_data` 会按本地时区和 UTC 各发一份，两个对象几 KB，
比继续猜便宜。

**证据边界**：这条路径属于「线上实测命中」——对象回读得到、详情页能显示轨迹。它不等于
「官方客户端 7.4.01/7.4.02 就是这么写的」；跨小时记录（比如 22:59 开跑、23:03 结束）
没有对照样本，小时目录到底按哪个时刻切，目前是按「提交窗口起点」推的。

## 外层 10 个键

真机对象顶层只有这 10 个键，**没有** `extension_json`：

| 键 | 内容 |
|---|---|
| `rrid` | 记录 id 的字符串 |
| `uuid` | 本次上传的 uuid 字符串 |
| `uid` | 账号 uid 字符串 |
| `run_data` | 内层对象，见下 |
| `fixed_point_json` | 主报文那层 envelope 的原样字符串（含 `fivePointJson` 内层串、`useZip`、`runAreaId`、`geoFencesJson`、`freedomShowFence`），**不是**裸点位数组 |
| `segment_json` | 分段配速数组（起步段 + 20 秒窗口 + 收尾段） |
| `speed_json` | 速度桶数组 |
| `step_freq_json` | 步频桶数组 |
| `laps_json` | 圈数组（每 1000m 一圈，末圈 `isFullLap=false`） |
| `runFaceCheck` | 空字符串 |

## 编码层级

每个值都是 **gzip → base64（STANDARD）**，不是明文 JSON。服务端要先解 base64 再 gunzip
才读得到内容。字符串类字段（`rrid`、`uuid`、`uid`、`runFaceCheck`）也是「字符串 → gzip →
base64」，不是裸字符串。

## `run_data` 内层

`run_data` 自己也是 gzip+base64，解出来是一个两键对象：

```json
{"allLocJson": "<JSON 数组的序列化字符串>", "useZip": false}
```

`allLocJson` 是**字符串**，内容是点位数组的 JSON 序列化（28 字段/点），不是数组本身。
`useZip` 是布尔，目前恒 `false`。

## 未覆盖：`/api/v1/obs/error`

这个错误上报端点没有还原，属于**未覆盖项**。看到它、或者看到任意 HTTP 200，都不能单独
推出哪一阶段成功或失败——至少要分开看这四段：

1. 换临时签名 URL（`/api/obs/temporary/url`）失败
2. 对象 `PUT` 失败（签名过期、桶权限）
3. `PUT` 返回 200 但**回读**取不到内容（key 写错、桶策略、分片静默失败）
4. 对象落盘成功，但服务端解析失败（编码层级、字段形状不对）

本实现目前只对第 2、3 段有明确日志（`PUT` 状态码 + 回读校验），第 4 段只能靠「详情页
没有轨迹 / 记录里只剩标量」反推。

## 合成 schema 样例（不含任何真实轨迹或账号）

只用来核对形状与编码层级，不用于证明字段单位或计分行为：

```json
{
  "rrid": "gz:<base64 of gzip(\"1234567\")>",
  "uuid": "gz:<base64 of gzip(\"00000000-0000-4000-8000-000000000000\")>",
  "uid": "gz:<base64 of gzip(\"10000000\")>",
  "run_data": "gz:<base64 of gzip({\"allLocJson\":\"[{\\\"totalDis\\\":0.0,...}]\",\"useZip\":false})>",
  "fixed_point_json": "gz:<base64 of gzip(\"{\\\"fivePointJson\\\":\\\"[...]\\\",\\\"useZip\\\":false,...}\")>",
  "segment_json": "gz:<base64 of gzip([{\"avgSpeed\":3.05,\"avgStep\":62,...}])>",
  "speed_json": "gz:<base64 of gzip([...])>",
  "step_freq_json": "gz:<base64 of gzip([...])>",
  "laps_json": "gz:<base64 of gzip([...])>",
  "runFaceCheck": "gz:<base64 of gzip(\"\")>"
}
```
