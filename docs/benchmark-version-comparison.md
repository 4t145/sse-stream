2026-09-11：当前工作区、crates.io 发布版与 sse-core 的全部 benchmark 对比

此报告中的“当前”对应短消息优化前的工作区快照；后续实现与复测见 [短消息优化结果](short-message-optimization.md)。

当前版本相对 crates.io 发布版在全部 28 个解码和 5 个编码场景中都更快，且三轮均超过 3% 的耗时改善。解码加速倍数的三轮中位值为 **1.25–10.55×**，编码为 **1.62–6.48×**。

与 sse-core 相比，尚未全面领先。按三轮中位值和 ±3% 耗时带粗分，是 18 项领先、3 项接近、7 项落后；要求三轮方向都一致且每轮差距都超过 3%，则是 **13 项均领先、5 项均落后、10 项接近或有波动**。这只是本机复测的一致性描述，不是统计显著性检验。

最明确的优势是 Unicode 大载荷：当前速度是 sse-core 的 **4.33–5.39×**；大小消息混合的 JSON / 多行数据是 **1.27–1.71×**。最值得优先优化的是短单行 JSON，以及高频小 data 消息。

本次版本和测试口径：

- 当前：版本号仍是 0.2.6 的工作区快照，包含尚未发布的全部优化；SHA-256 和 Git HEAD 在 manifest.json 中。这里衡量的是工作区相对正式发布包的总变化，不能把收益全部归因于上一轮的两项改动。
- 发布版：[sse-stream 0.2.6](https://docs.rs/crate/sse-stream/0.2.6)，2026-09-03 发布；对手：[sse-core 0.2.3](https://docs.rs/crate/sse-core/0.2.3)，2026-07-31 发布。重新获取 crates.io 索引确认版本，并用索引 checksum 校验实际下载包。
- 三者使用默认 features。当前默认启用 memchr、simdutf8；发布包保持它自身的默认配置。依赖由同一 Cargo.lock 锁定，避免把旧依赖版本差异混入对比。
- 完整复用现有 benches/scenarios/mod.rs 的 28 个解码、5 个编码 fixture，数据量和切块方式不变。正式计时前，三者全部事件数量、内容通过独立参考解析器校验；两版编码输出逐字节一致。
- 解码入口：当前 SseByteStream::new，发布版 SseStream::from_bytes_stream，sse-core SseStream::new。发布版没有 SseByteStream，因此使用它公开提供的字节流入口，包含该入口本身的适配成本。
- Intel Core i5-13600K，固定逻辑 CPU 2；Rust 1.98.0，Criterion 0.8.2，标准 release 优化，无额外 RUSTFLAGS / LTO。每项每轮 40 samples、200 ms 预热、1 s 目标测量时间，三轮顺序分别为 current/published/core、published/core/current、core/current/published。共 282 组测量，串行计时约 441 秒；慢项目实际测量时间可超过 1 s。现有仓库默认是 100 samples、10 s，本次缩短单轮并增加轮换复测。
- 解码计入输入 Bytes 克隆和消费/释放所有事件的开销；编码沿用 iter_batched/LargeInput，输入 Sse 克隆不计时。不含真实网络、JSON 反序列化、应用排队或 RSS 测量。JSON 行场景使用 data: {JSON} 的 SSE 封装，不是把原始 NDJSON 交给 SSE 解析器。
- CPU 未锁频，governor 为 powersave。小差距和三轮排名反转的项目应保留不确定性；不使用这些数据声称真实网络延迟改善。

下表绝对耗时取三轮 Criterion mean 点估计的中位值；加速倍数取三轮内“对手耗时 / 当前耗时”比值的中位值，因此可能与两个显示耗时直接相除略有不同。倍数大于 1 表示当前更快，2× 表示耗时减半。

解码耗时单位：毫秒 / 完整 fixture，越小越快。倍数 = 对手耗时 ÷ 当前耗时，大于 1 表示当前更快。

| 场景 | 当前 ms | 发布版 ms | sse-core ms | 当前/发布版速度 | 当前/core速度 |
|---|---:|---:|---:|---:|---:|
| `small_data_whole_stream_chunk` | 4.072 | 5.043 | 3.474 | 1.29× | 0.86× |
| `small_data_tcp_chunks` | 4.013 | 5.196 | 3.507 | 1.30× | 0.87× |
| `small_data_event_chunks` | 4.629 | 6.143 | 4.676 | 1.32× | 1.02× |
| `small_data_4b_chunks` | 11.062 | 16.000 | 9.955 | 1.46× | 0.90× |
| `metadata_events_tcp_chunks` | 4.788 | 8.080 | 5.162 | 1.77× | 1.08× |
| `multiline_data_tcp_chunks` | 2.973 | 4.674 | 3.142 | 1.55× | 1.10× |
| `keepalive_whole_stream_chunk` | 1.030 | 2.236 | 1.164 | 2.20× | 1.16× |
| `keepalive_tcp_chunks` | 1.051 | 2.126 | 1.228 | 2.02× | 1.16× |
| `keepalive_line_chunks` | 3.595 | 7.457 | 3.331 | 2.07× | 0.94× |
| `keepalive_4b_chunks` | 10.948 | 22.002 | 9.926 | 2.03× | 0.94× |
| `large_data_event_chunks` | 0.083 | 0.878 | 0.100 | 10.55× | 1.16× |
| `large_data_tcp_chunks` | 0.131 | 1.045 | 0.138 | 7.75× | 1.03× |
| `medium_data_10b_chunks` | 1.794 | 3.467 | 1.825 | 1.90× | 1.01× |
| `json_line_tcp` | 1.099 | 1.578 | 0.827 | 1.43× | 0.75× |
| `json_metadata_after_tcp` | 1.622 | 2.182 | 1.761 | 1.35× | 1.06× |
| `json_crlf_tcp` | 2.343 | 2.884 | 2.434 | 1.25× | 1.06× |
| `large_utf8_tcp` | 0.231 | 1.964 | 1.003 | 8.30× | 4.33× |
| `large_utf8_event` | 0.188 | 1.677 | 0.999 | 8.92× | 5.39× |
| `large_json_utf8_tcp` | 0.245 | 1.995 | 1.084 | 8.16× | 4.44× |
| `large_json_escaped_tcp` | 0.132 | 1.056 | 0.144 | 7.99× | 1.09× |
| `large_json_base64_tcp` | 0.127 | 1.002 | 0.135 | 7.79× | 1.04× |
| `mixed_json_tcp` | 0.589 | 0.989 | 0.740 | 1.68× | 1.27× |
| `mixed_json_whole` | 0.524 | 1.139 | 0.745 | 2.19× | 1.45× |
| `mixed_multiline_tcp` | 0.495 | 1.036 | 0.779 | 2.09× | 1.57× |
| `mixed_multiline_whole` | 0.479 | 0.966 | 0.823 | 2.02× | 1.71× |
| `alternating_sizes_tcp` | 0.139 | 1.114 | 0.151 | 7.87× | 1.07× |
| `varied_sizes_tcp` | 0.036 | 0.241 | 0.033 | 6.88× | 0.95× |
| `comment_crlf_tcp` | 0.470 | 0.673 | 1.234 | 1.48× | 2.78× |

编码耗时单位：纳秒 / 单个事件，输入 Sse 克隆不计时。sse-core 无编码 API。

| 场景 | 当前 ns | 发布版 ns | sse-core | 当前/发布版速度 |
|---|---:|---:|---:|---:|
| `small_json` | 14.7 | 93.3 | N/A | 6.48× |
| `metadata_json` | 72.1 | 115.4 | N/A | 1.62× |
| `large_ascii` | 664.7 | 1251.2 | N/A | 1.88× |
| `large_utf8` | 812.8 | 1284.4 | N/A | 1.64× |
| `empty_with_retry` | 47.5 | 96.9 | N/A | 2.03× |

解码表中的 ms 是处理整个 fixture 的耗时，不能直接跨不同行比较；各场景的字节数、块数和事件数见 [preflight.csv](bench-version-comparison-2026-09-11/preflight.csv)。例如小 data 场景为 100,000 个事件，大载荷场景通常为 64 个事件。TCP 表示 fixture 按 1460 B 切块，不代表执行了 TCP 网络请求。sse-core 只提供解码能力，编码项记为 N/A，未加入手写编码器替代它。

三轮均落后于 sse-core 的项目如下，耗时增加百分比以 sse-core 为基准：

| 场景 | 当前耗时增加（中位值） | 三轮范围 |
|---|---:|---:|
| `small_data_whole_stream_chunk` | +15.9% | +14.3% 至 +17.2% |
| `small_data_tcp_chunks` | +14.7% | +14.4% 至 +15.2% |
| `small_data_4b_chunks` | +11.1% | +3.4% 至 +16.8% |
| `keepalive_line_chunks` | +6.2% | +4.3% 至 +13.0% |
| `json_line_tcp` | +32.9% | +24.0% 至 +38.8% |

下列项目接近边界或排名有波动，不根据中位值单独宣称稳定胜负。范围是三轮观测值的最小/最大值，不是置信区间：

| 场景 | 当前/core 速度倍数的三轮范围 | 判断 |
|---|---:|---|
| `small_data_event_chunks` | 0.99–1.03× | 三轮均在 ±3% 耗时范围内 |
| `metadata_events_tcp_chunks` | 0.96–1.14× | 波动或接近边界 |
| `keepalive_4b_chunks` | 0.87–1.12× | 波动或接近边界 |
| `large_data_event_chunks` | 0.99–1.37× | 波动或接近边界 |
| `large_data_tcp_chunks` | 0.99–1.12× | 波动或接近边界 |
| `medium_data_10b_chunks` | 1.00–1.04× | 波动或接近边界 |
| `json_crlf_tcp` | 0.99–1.07× | 波动或接近边界 |
| `large_json_escaped_tcp` | 1.02–1.09× | 波动或接近边界 |
| `large_json_base64_tcp` | 1.03–1.06× | 波动或接近边界 |
| `varied_sizes_tcp` | 0.88–1.09× | 波动或接近边界 |

结合代码变化，发布版仍逐字节找换行、先拼完整行、再进入字段解析，并通过 VecDeque 暂存一个块里解析出的事件。当前版本的快速扫描、直接字节流入口、data 缓冲转移和 SIMD UTF-8 校验，与大载荷收益方向一致；编码预分配减少了扩容。这里是代码层面的解释，本次三方对比没有单独隔离各项改动，不能据此分摊每项贡献。

下一步优先级：先处理 `json_line_tcp` 的约 33% 差距，再处理 `small_data_whole_stream_chunk` / `small_data_tcp_chunks` 的约 15–16% 差距，随后关注 `small_data_4b_chunks` 的碎片处理成本和按行到达的心跳。大 ASCII TCP、复杂尺寸变化等项目应先消除测量波动再决定改动；Unicode 与大小混合消息已有明确优势，需要作为回归约束保留。

完整可复现材料：

- [summary.csv](bench-version-comparison-2026-09-11/summary.csv)：全部 33 项耗时、MiB/s、加速倍数、三轮范围和一致性分类。
- [rounds.csv](bench-version-comparison-2026-09-11/rounds.csv)：全部 282 组测量的 mean、median、mean 的 Criterion 95% bootstrap 区间。
- [manifest.json](bench-version-comparison-2026-09-11/manifest.json)：版本、索引记录、下载包校验、工作区源码哈希、环境及参数。
- [experiment.zip](bench-version-comparison-2026-09-11/experiment.zip)：独立 Cargo 工程、当前源码快照、依赖锁、fixture、预校验程序、运行/汇总脚本、三轮日志和原始 sample.json / estimates.json。解压后按 README.md 运行即可复测。

本次只新增对比报告和复现材料，库代码及现有 benchmark 保持本次任务开始时的状态。
