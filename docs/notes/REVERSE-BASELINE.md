# 逆向基线修正（7.4.20，2026-11-23）

对 `com.zjwh.android_wh_physicalfitness` **7.4.20（code 411）** 与 **7.4.01（code 409）**
做了三路静态逆向（Dart AOT / 原生 so / 壳与 dex）+ 版本差分。完整报告在
`/root/work/msworld/运动世界校园-7.4.20-逆向报告.md`（不在仓库内）。

本文只记**对仓库内既有结论的修正**，避免继续按错的基线推理。

## 一、撤回的三条旧结论

| 旧说法（出自 `运动世界校园-逆向报告.md`，v7.3.11） | 7.4.20 实测 |
|---|---|
| 「App 只有 38 个端点」 | 实际 **58**（7.3.11）/ **74**（7.4.20）。旧统计漏了 `/api/<模块>/v<版本>/…` 整类，`/api/v70260/runnings/save/record` 就在其中 |
| 「App 从不用 `v70320`」 | `classes.dex` 明文里有 `v70320/runnings/get_one_record`。Dart 层用 `v70260`，两套并存。**pace-forge 用 v70320 不算错**，但改回 v70260 需一次真机验证 |
| 「DEX 只有易盾壳，业务全在 libapp.so」 | dex 前 66 KB 是壳的 30 个类，**其后 73 MB 是明文主体**：72,125 个类、245 条 API 字面量、完整 Room DDL 可直接读 |

## 二、`crypto/consts.rs` 的出处更正

旧注释写「全部数值来自对 `libnetseckit-4.2.4.so` 的静态分析」。把该 so 的 `.text`
全量反汇编（303,458 槽位）后：

- 三段 KDF 的 `ror #8` / `#24`、`lsr#24 + lsl#8 + eor` 组合：**0 命中**
- MD5 K 表 `d76aa478`、IV `67452301`：**0 命中**
- `SIGN_SALT` / `md5_key` / `keyData*` / `d,h,k,p,t`：**0 命中**
- 236 个导出里唯一业务导出是 `JNI_OnLoad`

它是易盾 NetSecKit 的通用密码学 JNI 后端（SM2 完整曲线、rijndael、LibTomMath、
base64、zlib、jsoncpp、`ptrace` 反调试）。**这些常量的真实来源未定位**，
现有值按【实机联调】这一级引用，不要再说「从 so 里读出来的」。

## 三、7.4.20 相对 7.4.01 的真实变化

| 项 | 结论 |
|---|---|
| `libnetseckit-4.2.4.so`、`libswsport.so` | **md5 完全相同**，加密/签名层没变 |
| 提交端点 `v70260/runnings/save/record` | **一字未动** |
| 极验 `GT_KEY` | 与 `crypto/geetoken.rs` 逐字节一致 |
| 组件表（329/60/20/30） | 零增零减 |
| 权限 | 71 → 70，只少了 `MANAGE_EXTERNAL_STORAGE` |
| 端点 | +27/−11，主体是版本号平移（v70300→v70380 等），新增 `v70410/user/getShieldList` |
| **新增** | 学校配速/步频区间的**点级**校验：`isPaceOutOfRange`、`isStepMaxOutOfRange`、`isStepMinOutOfRange`、`paceLimitLines`、文案「*学校要求配速范围在 / *学校要求步频范围在」 |
| 人脸 | 商汤模型下线，改**百度 + 支付宝**（`v70380/run/alipay/auth`、`v70103/user/face/upload`） |
| 旧报告漏掉的端点 | `api/runnings/upload/originalSign` |

## 四、仍然只有推断的（别当结论用）

1. OBS key `run_data/{YYYYMMDDHH}/{uuid}.json` —— **线上探测**得出（13 候选逐个 GetObject），
   Dart 层没有 `run_data` 字面量。不是客户端代码依据。
2. `segment_json` / `extension_json` / `runFaceCheck` 留空 —— 7.3.11 真机确认，**7.4.20 未复核**。
3. `avgStep` 是段步数还是步频 —— issue #12 未决。
4. `131072` 状态位 —— 只有「首段恒打」的观测。
5. `speedStandard=900` / `intervalStepModel=0` —— 单一学校样本的常量。

## 四之二、真机采样点距差 3 倍（别再按单一"真机值"改代码）

两条真机记录的采样点距并不一致：

| 样本 | 距离 | 点数 | 点距 |
|---|---:|---:|---:|
| rrid 1362680775 | 1.93 km | 406 | **4.75 m** |
| rrid 1328288734 | 1683.83 m | 114 | **14.77 m** |

同一个 App、同一套协议，点距差 3 倍——说明采样点距跟设备 GPS 更新率/系统调度有关，
**不是一个固定常数**。pace-forge 生成 13.7 m/点，贴着上面第二条，属正常范围。

历史教训：旧逆向报告只看到 4.75 m 那一条，就把 `sample_step_m` 从 10 改成 5
（`6aa841e`），后来又发现 5 秒采样下点距本来就是 13~14 m —— 改的是规划步长，
不是上报点距，两件事别混。**以后要改采样，先确认改的是 `sample_step_m`（几何）
还是 `SAMPLE_INTERVAL_SEC`（上报节奏），并至少拿两条不同设备的真机记录对照。**

## 五、201「app 参数数据异常」

客户端里**不存在**这条文案（Dart 两种编码 + 73 MB dex 明文穷举，0 命中，也没有
错误码字典），纯服务端业务码。三个候选成因：

1. ~~极验/DeepKnow 收据~~ —— 已排除：同类工具 `NekoSportsWorldTool`（7.3.70 协议）
   的 `checkGeeUse`/`geevalidate` 同样只挂在**登录**链路。
2. **点级/桶级区间校验口径** —— 最可能。`preflight` 原来只校验平均配速
   （`PACE_OUT`）与平均步频（`CADENCE_OUT`），现已补 `PACE_OUT_BODY` /
   `BUCKET_PACE_OUT` / `BUCKET_CADENCE_OUT` 三条对冲提示。
3. `runFaceCheck` 为空的语义 —— 无证据，暂不动。

## 六、设备侧结论（同一轮验证）

- Shizuku 通道可用（`uid=2000(shell)`），但 App 设了 `FLAG_SECURE`：截图与无障碍读屏都拿不到界面。
- DSH 虚拟屏 `dsh-background` 的 `canHostTasks=false`，App 启动即退（Flutter 依赖 GPU 合成）。
- App **不打印业务请求日志**（`logcat --pid` 全量只有字节跳动/快手广告 SDK 的输出）。
  「`ApiService` 会打全量参数」不成立——那是类名，不是日志 tag。
- 无 root → `frida` / `tcpdump` 都不可用。
- 抓包期间设备上有 Clash 在跑，`oplusDnsOpenProxy` 连接被拒（`errno=111`）。
  **出口走代理会改变服务端看到的 IP**，排查 201 时必须算进这一项。
