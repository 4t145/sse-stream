本报告基于 2026-09-10 的工作区，包含当时未提交的优化；不是仅基于 HEAD `06ae23b`，也不代表 crates.io 已发布版本的性能。所有解析器实验都在 `/tmp/sse-bench-study-5ci1q191` 中完成，现有源码、测试、README 和 benchmark 未被改动。

结论：目前值得优先优化的是完整小事件的处理路径、每个输入块的处理成本，以及按输入形态选择换行扫描方式。已有架构可以把主要落后场景拉到持平，部分场景可以明显超过 `sse-core`。单纯扩大 SIMD 使用范围或加内联提示，都不能保证全场景变快。

**测量依据与限制**

- 对照为锁定的 `sse-core 0.2.3`；核对了缓存 `.crate` 的 SHA-256 与 Cargo.lock 一致，读取的 decoder/stream 源码与归档一致。
- Intel Core i5-13600K，固定逻辑 CPU 2，Rust 1.98.0，默认 `memchr` feature，Criterion 0.8.2。两个库使用相同输入、编译配置和运行时。
- 先在当前代码快照上跑完整基线，再分别测 7 个单因素原型，最后将当前代码快照作为独立依赖，与组合原型、sse-core 编译进同一个 benchmark 程序。
- 最终三方复测每组 40 个样本，warm-up 0.3 s、目标 measurement 1.5 s；Criterion 对较慢场景会自动延长。原有 13 个场景之外，增加 16/32/64/128 字节 payload、CRLF 和中文/emoji，共 19 个场景。
- 表中统一使用均值；比值小于 1 表示耗时更短。原始均值、95% 置信区间和样本数见 [measurements.csv](bench-study/measurements.csv)。小于约 3% 的点估计差异，尤其置信区间重叠时，按接近持平解读。
- 单机、单次最终复测、没有锁定 CPU 频率，也没有硬件性能计数器采样。单因素实验适合筛选方向，收益归因不能等同于火焰图证明，亦不能把各项百分比相加。正式合并前需要多轮交错复测和其他架构验证。

当前已有结果中部分是 quick run，且不同场景测量时间不同；旧 `async_parsing_*` 目录对应旧负载，不混入本次比较。当前 bench 已排除 retry，以免两库输出事件数量不同，这一点是正确的。

| 场景 | sse-core / ms | 当前实现 / ms | 实验原型 / ms | 当前 / core | 原型 / core |
| --- | ---: | ---: | ---: | ---: | ---: |
| `keepalive_4b_chunks` | 9.2437 | 10.0772 | 8.9779 | 1.090 | 0.971 |
| `keepalive_line_chunks` | 3.1814 | 3.3286 | 3.1099 | 1.046 | 0.978 |
| `keepalive_tcp_chunks` | 1.1160 | 1.0262 | 1.1185 | 0.920 | 1.002 |
| `keepalive_whole_stream_chunk` | 1.0764 | 0.9846 | 1.1173 | 0.915 | 1.038 |
| `large_data_event_chunks` | 0.0929 | 0.0890 | 0.0890 | 0.957 | 0.958 |
| `large_data_tcp_chunks` | 0.1296 | 0.1364 | 0.1289 | 1.053 | 0.995 |
| `medium_data_10b_chunks` | 1.7464 | 1.8079 | 1.6955 | 1.035 | 0.971 |
| `metadata_events_tcp_chunks` | 4.6328 | 4.1531 | 3.7530 | 0.896 | 0.810 |
| `multiline_data_tcp_chunks` | 2.7692 | 2.7461 | 2.3764 | 0.992 | 0.858 |
| `small_data_4b_chunks` | 9.0157 | 10.0089 | 8.9080 | 1.110 | 0.988 |
| `small_data_event_chunks` | 4.5214 | 4.3179 | 3.7151 | 0.955 | 0.822 |
| `small_data_tcp_chunks` | 3.2996 | 3.8361 | 2.8451 | 1.163 | 0.862 |
| `small_data_whole_stream_chunk` | 3.2767 | 3.9155 | 2.7895 | 1.195 | 0.851 |

原型使小消息整流块/TCP 块的耗时相对当前实现降低约 29%/26%，相对 core 降低约 15%/14%；metadata 和多行 data 相对 core 分别降低约 19%/14%。4 B 小消息、4 B keepalive、10 B 中等消息、大消息 TCP 分片均接近 core。完整大消息继续保持接近原有水平。

代价也明确：原型的整流块/TCP 块 keepalive 相对当前实现退化 13.5%/9.0%，因此它是用于验证方向的原型，不能直接作为“全场景优化完成”的补丁。

**原因一：固定扫描前 16 字节，使完整短行付出多余成本**

位置：[find_line_end](../src/stream.rs#L560)。当前实现先对前 16 字节逐字判断 CR/LF，没有命中才调用 memchr2；对每个新切片都做这一步。短 data 行的换行常常就在这个范围内，批量输入也用不上向量搜索；长数据分片则每个块都多做一次标量前缀扫描。对照库在识别字段名之后用 memchr2 扫描 value。

`memchr2` 本身提供优化的向量搜索，但这并不意味着所有短切片都适合调用它。[官方说明](https://docs.rs/memchr/latest/memchr/fn.memchr2.html)

单因素试验将标量前缀设为 0，小消息整流块/TCP 块相对首轮基线约减少 14%/12%，但短碎片与按条输入的 keepalive 出现退化。改为“切片长度 ≤16 时标量，否则直接 memchr2”的策略，小消息及多行 data 仍有收益，短碎片表现更好。组合结果说明，单一阈值仍未照顾好批量注释行。

建议：将扫描策略区分为短前缀、data value 连续区间、comment 三种调用环境。首先保留当前已表现较好的短 comment 扫描路径；对长 value 使用 memchr2；用不同长度与终止符位置的微基准决定阈值，不根据当前固定的 JSON 示例长度写特殊判断。

**原因二：完整单行 data 仍走累积、补分隔符、分发的通用流程**

位置：[push_data_line](../src/stream.rs#L232)、[完整 data 分支](../src/stream.rs#L448)、[dispatch](../src/stream.rs#L202)。`data: value\n\n` 会先验证 UTF-8、复制进 data_buf、push 换行，再回到循环扫描空行、pop 换行、转移 Vec 并为下个事件分配缓冲区。

验证过的改法：当前没有已累积字段、没有待补行，并且一个完整 data 行和 LF 空行都在当前切片内时，直接验证 value、生成拥有所有权的 String、消费两个换行并返回。CR、CRLF、多行、metadata、BOM、碎片输入继续走通用路径。

单独加这条路径，小消息整流块/TCP 块/按事件分块相对首轮基线减少约 21%/19%/18%。组合原型进一步改善。收益来自更短的控制流程和更紧凑的分配，不是消除了一个原本存在的整 payload 二次复制：当前 dispatch 的 Vec→String 已经是所有权转移。

分配计数也证实，不应把稳态说成“两次 malloc/事件”。10 万条 9 B payload：当前实现约 100002 次 alloc + 1 次 realloc，输出 data capacity 总和 1.8 MB；原型 100000 次 alloc、0 次 realloc，capacity 总和 0.9 MB。主要是减少容量浪费和状态操作，分配次数的数量级没有变化。

**原因三：极碎输入会放大每块状态维护和输入对象成本**

位置：[SseByteStream::poll_next](../src/stream.rs#L630) 与 [parse_buf](../src/stream.rs#L492)。当前将保留的 data 和新收到的 data 分成两套解析分支；新块先放局部变量，解析后再决定是否存入 self.data，并在事件产出时判断是否立即清空。这些操作在 4 B/10 B 输入上被重复数十万次。

原型统一成：先解析 self.data，耗尽后从上游获取下一块放入同一槽位，事件直接返回，空块的清理由下一轮处理。单因素试验中，4 B 小消息与 4 B keepalive 相对首轮基线约减少 9%/10%；最终组合将它们从慢约 11%/9% 拉到接近持平。

返回结构大小也值得后续关注：本机 `Sse` 为 88 B、`sse_core::SseEvent` 为 64 B，`parse_chunk` 的 Result 为 96 B；但大小本身不能证明实际复制成本，编译器可能优化掉搬运。试验里 `#[inline]`、`#[inline(always)]` 都未产生稳定的碎片收益，部分场景反而明显退化，不建议盲目添加。

另一个实测约束来自 benchmark 输入本身：[bench 中的 iter/cloned](../benches/bench.rs#L104)。预构造 Vec 在计时外，但每个 Bytes 的 clone/drop 在计时内，共享数据块涉及原子引用计数。当前启用的 tokio-stream 0.1.19 默认 feature 没有 rt，`iter` 每消费 32 项会返回一次 Pending 并 wake；源码见本机 registry 的 `tokio-stream-0.1.19/src/iter.rs`。因此该测试没有网络，却有确定的人工让出行为。

独立诊断程序固定同一 CPU，预热 3 次、测量 21 次取中位数，得到：

| 输入 | 块数 | 仅消费 Bytes / ms | 仅消费借用切片 / ms | 当前解析 Bytes / ms |
| --- | ---: | ---: | ---: | ---: |
| 小消息，4 B | 425000 | 6.360 | 0.480 | 12.003 |
| keepalive，4 B | 650000 | 9.820 | 0.902 | 12.638 |
| 中等消息，10 B | 105421 | 1.580 | 0.112 | 2.355 |

这里“仅消费 Bytes”已经相当于对应解析总时长的约 53%/78%/67%。改用 futures 的直接 ready 输入、保留 Bytes 时，仅消费耗时仍为 6.240/9.544/1.556 ms，说明这组诊断里主要固定成本是 Bytes 的克隆/释放，人工 Pending 次要。两库承担相同的输入成本，它不能单独解释两库差距，却会掩盖纯解析器的改进。不同类型会改变代码生成，不能简单相减得到精确解析耗时，也不能把这里的绝对时长与 Criterion 表直接拼接。

建议同时保留两种 bench：借用 &[u8]、同步 decoder/ready stream 用于定位解析成本；Bytes + 可控 Pending 用于覆盖实际集成成本。不要只更换输入迭代器后就把数字下降当成库性能提升。

**原因四：历史最大容量持续传给后续小事件，现有负载没有覆盖**

位置：[dispatch 的 Vec::with_capacity](../src/stream.rs#L219)。注释说保留 allocation，实际是将旧 allocation 交给输出，再按相同 capacity 申请一个新 allocation。由于 data 后追加一个换行，首次容量恰好等于数据长度时还可能触发倍增扩容。

实测在同一连续输入块内放一条 40000 B data，随后 1000 条 1 B data：当前实现每条后续小消息的 data capacity 仍为 80000 B；1001 条输出的 capacity 累计 80080000 B，分配/扩容申请字节累计 80200000 B。对照库也存在相同现象。当前消费者立即丢弃事件，因此这些数字不是同时驻留的 RSS；若消费者保存所有输出，capacity 总和才对应那些仍持有的输出缓冲容量。

完整单行快捷路径的原型在这个特定连续 LF 输入中，将输出 capacity 总和降到 41000 B。但碎片事件和多行事件仍走原缓冲策略，问题并未全面解决。

建议另外做容量策略优化：预留时将分隔符计入需求，避免已知完整长度后 push 触发翻倍；下一事件延迟分配，或者限制预留容量并根据最近负载回落。保留大消息连续到达时的容量提示，以免将尾部容量问题换成每条大消息反复扩容。通过“大→小”“小→大→小”“大小交替”和消费者保留输出的负载，同时考察耗时、realloc 次数、分配字节及峰值内存。

**可执行的优化顺序**

1. 先固定评测口径：保留这 13 个场景，增加字段内容的规范化校验，在计时外确认两库输出；报告 events/s、bytes/s 和 ns/chunk。加入不同 payload 长度、随机分片、非 ASCII、CRLF 和可控 Pending。每个版本交错重复至少三轮，保留 CI 和编译配置。
2. 优先做两个范围明确的变更：统一 byte stream 的块处理路径；加入完整单 data 行的快捷路径。保持现有公开 String API、错误处理、分片恢复与多段 Buf 支持。分别测、再组合测。
3. 单独调整扫描策略：将 data 的改进与 comment 的当前优势结合。验收目标是消除原型已知的 keepalive 9%～14% 退化，同时保住小消息 20% 以上的自身收益；这是目标，不是当前已验证结论。
4. 再优化容量策略，覆盖两库共同薄弱的“先大后小”真实负载。它比仅针对等长小 JSON 调整阈值，更有机会形成实际应用中的优势。
5. 若仍受大 payload UTF-8 验证限制，再试可选 SIMD 校验，按长度启用，并在验证失败时走标准库以保持现有 Utf8Error 的精确信息。尚未测量其收益，不承诺固定倍数。[simdutf8 的验证接口](https://docs.rs/simdutf8/latest/simdutf8/basic/fn.from_utf8.html)
6. 更激进的方案是另加 Bytes 输出或回调式借用 decoder：完整连续 payload 可减少复制/分配。现有 `Option<String>` 输出不能直接引用输入 Bytes，所以需要新增 API；碎片、多行拼接仍可能要复制，Bytes 切片也可能延长整块输入的存活时间。应把“调用方最终是否仍需 String”纳入对照，避免只把成本移给调用方。

不优先做：重新引入事件队列、为了凑块而等待网络数据、把 generic Buf 一概换成动态分发、无条件内联整个解析器、只开启 target-cpu=native 或 LTO 后只给自己报告收益。当前代码已经有 memchr、data 碎片直接追加、分割前缀识别、注释丢弃、按需返回事件和直接字节流适配，不应把已完成的优化重新当作本轮方案。

**验证结果和方案边界**

临时原型在 default、all-features、no-default-features 三组配置下，均通过现有 33 个 byte parser 测试、1 个编码测试，以及新增的 1 个分片等价性测试。等价性测试包含 15 类输入的每个单切点、1～64 B 固定分片，以及 100 组随机分片布局；覆盖空 data、多行、metadata、BOM、CRLF、UTF-8、非法字节与错误后的行为。没有运行网络集成测试、Miri 或长期 fuzz。

这些验证保证的是原型相对当前行为的回归检查，不能据此宣称完整 EventSource 规范一致性。两库对 retry、重复字段、未知字段、非法 UTF-8、id 跨事件状态等处理不同；扩展比较时应先限定共同语义，或者显式规范化输出，不能靠减少语义工作取得“提速”。[WHATWG SSE 解析与解释规则](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream)

补充负载的三方结果如下，其中 CRLF 不满足新增的 LF 单行快捷路径条件，仍有收益，说明改进不完全依赖那条快捷路径：

| 场景 | sse-core / ms | 当前实现 / ms | 实验原型 / ms | 当前 / core | 原型 / core |
| --- | ---: | ---: | ---: | ---: | ---: |
| `extra_payload_128b_tcp` | 1.8861 | 2.9095 | 1.7459 | 1.543 | 0.926 |
| `extra_payload_16b_tcp` | 1.6084 | 2.0969 | 1.4192 | 1.304 | 0.882 |
| `extra_payload_32b_tcp` | 1.9220 | 2.3524 | 1.5020 | 1.224 | 0.781 |
| `extra_payload_64b_tcp` | 1.8525 | 2.3614 | 1.6040 | 1.275 | 0.866 |
| `extra_small_crlf_tcp` | 3.7467 | 3.8836 | 3.1743 | 1.037 | 0.847 |
| `extra_utf8_tcp` | 3.7233 | 4.3245 | 2.9754 | 1.161 | 0.799 |

[prototype.patch](bench-study/prototype.patch) 是本次组合原型相对当前源码快照的差异，仅供审阅，未应用到工作区。它保留了上文提到的 keepalive 退化，用于明确展示“哪些收益已验证，哪些仍需调整”。[manifest.json](bench-study/manifest.json) 记录了版本与源码指纹。

临时研究副本保留了 `run_variants.py`、`run_variants2.py`、三方比较的 bench、allocation/overhead probe、等价性测试与所有日志。复跑三方比较可在该副本中执行：

```bash
env CARGO_TARGET_DIR=/home/atlas/Github/4t145/sse-stream/target \
  CRITERION_HOME=/tmp/sse-bench-study-5ci1q191/results/verify-repeat \
  taskset -c 2 cargo bench --offline --bench bench -- \
  --sample-size 40 --warm-up-time 0.3 --measurement-time 1.5 --noplot
```

分配和输入开销诊断的输出另存于 [diagnostics.txt](bench-study/diagnostics.txt)。临时目录可能被系统清理；长期保存的证据为本报告、原型补丁、原始测量表、诊断输出及 manifest。合并前应在目标代码版本上重建实验并补齐上述验收。
