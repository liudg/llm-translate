# Translation quality comparison

## rust-error  →中文
> error[E0502]: cannot borrow `x` as mutable because it is also borrowed as immutable

- **HY-MT1.5-1.8B** (252 ms): 错误[E0502]：无法将变量`x`视为可变的，因为它也被声明为不可变的。
- **Hy-MT2-1.8B** (286 ms): 错误[E0502]：无法将`x`视为可变的变量进行借用，因为它同时被当作不可变变量来使用
- **Hy-MT2-7B** (879 ms): 错误[E0502]：无法将`x`作为可变值使用，因为它已经被标记为不可变值了。
- **HY-MT1.5-7B** (814 ms): 错误[E0502]：无法将`x`借出用于可变用途，因为它已经被借出用于不可变用途了。

## tech-doc  →中文
> The function returns a future that resolves once all pending writes have been flushed to disk.

- **HY-MT1.5-1.8B** (208 ms): 该函数返回一个未来值，该值会在所有待写入的数据被成功写入磁盘后才会变得可用。
- **Hy-MT2-1.8B** (212 ms): 该函数返回一个未来值，该值在所有待写的数据被写入磁盘后才会得到解决。
- **Hy-MT2-7B** (769 ms): 该函数会返回一个未来值，一旦所有待处理的写入操作都被刷新到磁盘上，这个未来值就会变为实际值。
- **HY-MT1.5-7B** (751 ms): 该函数返回一个future对象，只有当所有待写的数据都被刷新到磁盘后，这个future对象才会最终得到执行结果。

## idiom-en  →中文
> It's not rocket science, but it's easy to drop the ball if you cut corners.

- **HY-MT1.5-1.8B** (158 ms): 这并不是什么复杂的操作，但是如果你偷工减料的话，就很容易出错。
- **Hy-MT2-1.8B** (178 ms): 这并非高深莫测的事情，但如果偷工减料的话，很容易出错。
- **Hy-MT2-7B** (541 ms): 这并非什么高深的技术，但若偷工减料，就很容易出错。
- **HY-MT1.5-7B** (633 ms): 这其实并不是什么高深莫测的事情，但如果你偷工减料的话，就很容易犯错。

## ui-string  →中文
> Unsaved changes will be lost. Do you want to continue?

- **HY-MT1.5-1.8B** (141 ms): 已保存的修改将会被丢失。您是否想要继续操作？
- **Hy-MT2-1.8B** (127 ms): 未保存的修改将会丢失。您是否要继续操作？
- **Hy-MT2-7B** (353 ms): 未保存的更改将会丢失。是否要继续？
- **HY-MT1.5-7B** (450 ms): 未保存的更改将会丢失。您确定要继续吗？

## word-en  →中文
> deprecated

- **HY-MT1.5-1.8B** (363 ms): 已不再被推荐使用
- **Hy-MT2-1.8B** (44 ms): 被废弃
- **Hy-MT2-7B** (104 ms): 已废弃
- **HY-MT1.5-7B** (130 ms): 已被弃用

## academic  →中文
> We propose a novel attention mechanism that scales linearly with sequence length while preserving the expressiveness of full attention.

- **HY-MT1.5-1.8B** (230 ms): 我们提出了一种新的注意力机制，该机制能够随着序列长度的增加而线性扩展，同时还能保持全注意力机制所具备的表达能力。
- **Hy-MT2-1.8B** (207 ms): 我们提出了一种新的注意力机制，该机制与序列长度成线性增长，同时保留全注意力模式的表现力。
- **Hy-MT2-7B** (744 ms): 我们提出了一种新的注意力机制，其复杂度与序列长度呈线性关系，同时仍能保持全注意力机制的表达能力。
- **HY-MT1.5-7B** (824 ms): 我们提出了一种全新的注意力机制，该机制能够随着序列长度的增加而线性扩展，同时仍保留完整注意力机制所具有的表达能力。

## npm-warn  →中文
> npm WARN deprecated inflight@1.0.6: This module is not supported, and leaks memory.

- **HY-MT1.5-1.8B** (217 ms): npm警告：inflight@1.0.6已不再被支持；该模块不可使用，并且会导致内存泄漏。
- **Hy-MT2-1.8B** (218 ms): npm WARN：已废弃的inflight@1.0.6模块——该模块不受支持，还会导致内存泄漏。
- **Hy-MT2-7B** (702 ms): npm警告：inflight@1.0.6已过时——该模块不再被支持，且会导致内存泄漏。
- **HY-MT1.5-7B** (793 ms): npm警告：inflight@1.0.6版本已被弃用——该模块目前不再受到支持，且会导致内存泄漏。

## news  →中文
> Shares of the chipmaker jumped 12% in after-hours trading on Tuesday after it raised its full-year revenue forecast to $48 billion, citing stronger-than-expected demand from data centers.

- **HY-MT1.5-1.8B** (301 ms): 这家芯片制造公司的股票在周二盘后交易中上涨了12%。该公司将全年的收入预测上调至480亿美元，并表示来自数据中心的需求比预期要强劲。
- **Hy-MT2-1.8B** (283 ms): 周二盘后交易中，这家芯片制造商的股价上涨了12%。该公司将全年营收预期提高至480亿美元，原因是数据中心的需求比预期更为强劲。
- **Hy-MT2-7B** (945 ms): 由于数据中心的需求强于预期，这家芯片制造商上调了全年的营收预期至480亿美元，其股票在周二盘后交易中上涨了12%。
- **HY-MT1.5-7B** (1074 ms): 这家芯片制造商在周二盘后交易中，其股价上涨了12%。原因是该公司将全年的收入预期上调至480亿美元，并表示数据中心的采购需求比预期要强劲。

## colloquial  →英语
> 这事儿说来话长，咱们边吃边聊吧。

- **HY-MT1.5-1.8B** (192 ms): It’s a story with a long history. Let’s continue talking while we eat together.
- **Hy-MT2-1.8B** (147 ms): This is a long story. Let’s talk while we eat.
- **Hy-MT2-7B** (409 ms): It’s a long story; let’s talk while we eat.
- **HY-MT1.5-7B** (467 ms): There’s a long story behind this, so let’s talk while eating.

## idiom-zh  →英语
> 他做事总是虎头蛇尾，让人很头疼。

- **HY-MT1.5-1.8B** (199 ms): He always starts things with great enthusiasm but ends them in a disappointing way, which really gives me a headache.
- **Hy-MT2-1.8B** (172 ms): He always starts things with great enthusiasm but ends them in a mess, which is really troublesome.
- **Hy-MT2-7B** (522 ms): He always starts things with enthusiasm but fails to see them through, which is very troublesome.
- **HY-MT1.5-7B** (565 ms): He always starts things with great enthusiasm but ends up failing to complete them, which is really troublesome.

## tech-zh  →英语
> 该接口在高并发场景下会出现偶发的超时，需要排查连接池配置。

- **HY-MT1.5-1.8B** (239 ms): This interface may occasionally experience timeout issues under high-concurrency conditions. It is necessary to investigate the configuration of the connection pool.
- **Hy-MT2-1.8B** (198 ms): This interface may experience occasional timeouts under high-concurrency conditions; it is necessary to investigate the connection pool configuration.
- **Hy-MT2-7B** (682 ms): This interface may experience occasional timeouts in high-concurrency scenarios; it is necessary to check the configuration of the connection pool.
- **HY-MT1.5-7B** (675 ms): In scenarios with high concurrency, this interface may occasionally experience timeouts; it is necessary to check the configuration of the connection pool.

## term-zh  →英语
> 划词翻译

- **HY-MT1.5-1.8B** (550 ms): Translated captions/notes
- **Hy-MT2-1.8B** (59 ms): Translated words
- **Hy-MT2-7B** (743 ms): Word-by-word translation
- **HY-MT1.5-7B** (165 ms): Word-by-word translation

## copy-zh  →英语
> 一键部署，零门槛上手，让每个人都能用上本地大模型。

- **HY-MT1.5-1.8B** (172 ms): One-click deployment, with no initial investment required. Anyone can utilize local large models easily.
- **Hy-MT2-1.8B** (168 ms): One-click deployment, with no barriers to entry, allowing everyone to use local large models.
- **Hy-MT2-7B** (505 ms): One-click deployment with no barriers to getting started, enabling everyone to use local large models.
- **HY-MT1.5-7B** (455 ms): One-click deployment and zero-barrier access allow everyone to use local large models.

## slang-zh  →英语
> 这波操作直接给我整不会了。

- **HY-MT1.5-1.8B** (100 ms): This operation completely ruins everything for me.
- **Hy-MT2-1.8B** (98 ms): This operation completely confuses me.
- **Hy-MT2-7B** (271 ms): These actions have completely left me confused.
- **HY-MT1.5-7B** (286 ms): This series of actions completely confused me.

## formal-zh  →英语
> 根据相关规定，申请人应当在收到通知之日起十五日内提交补充材料。

- **HY-MT1.5-1.8B** (216 ms): According to the relevant regulations, the applicant must submit additional documents within fifteen days from the date of receiving the notification.
- **Hy-MT2-1.8B** (198 ms): According to relevant regulations, the applicant must submit additional documents within fifteen days from the date of receiving the notice.
- **Hy-MT2-7B** (614 ms): In accordance with relevant regulations, the applicant must submit supplementary materials within fifteen days from the date of receiving the notification.
- **HY-MT1.5-7B** (618 ms): In accordance with relevant regulations, the applicant must submit supplementary materials within fifteen days from the date of receiving the notification.
