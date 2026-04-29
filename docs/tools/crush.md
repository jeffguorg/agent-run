# Crush

## 配置入口

`Crush` 的配置分成两层：

- 全局配置目录
- 当前项目目录

对 `agent-run` 最关键的是这几个入口：

- `CRUSH_GLOBAL_CONFIG`
- `CRUSH_GLOBAL_DATA`
- `--data-dir`
- 当前工作目录下的项目级上下文文件

其中有一个容易踩坑的点：

- `CRUSH_GLOBAL_CONFIG` 指向的是配置目录，不是 `crush.json` 文件本身

也就是说，如果 launcher 要接管运行时配置，应该准备：

- 一个临时配置目录，里面放 `crush.json`
- 一个临时 data dir

而不是直接把某个 JSON 文件路径塞给 `CRUSH_GLOBAL_CONFIG`。

## 与 `agent-run` 的适配策略

`agent-run` 对 Crush 的适配目标是：

- 不改用户长期配置
- 按进程临时指定 provider / model
- 允许沿用 Crush 自己的项目级行为

当前推荐策略是：

1. 在缓存目录生成运行时配置目录
2. 把运行时 `crush.json` 写进去
3. 设置 `CRUSH_GLOBAL_CONFIG=<runtime-config-dir>`
4. 设置 `CRUSH_GLOBAL_DATA=<runtime-data-dir>`
5. 启动时附带 `--data-dir <runtime-data-dir>`
6. 直接执行 `crush <args...>`

这样做的结果是：

- provider / model / key 由本次启动隔离控制
- Crush 自己的数据文件落到隔离目录
- 当前项目目录仍然会被 Crush 当作项目上下文读取

## 协议与 provider 映射

Crush 本身可以接：

- OpenAI-compatible provider
- Anthropic-compatible provider

对 `agent-run` 来说，当前支持的映射是：

- `openai-chat-completions` -> `type: openai-compat`
- `anthropic` -> `type: anthropic`

`agent-run` 当前不会为 Crush 选择 `openai-responses`。如果 provider 同时支持多个协议，优先级是：

1. `openai-chat-completions`
2. `anthropic`

## 运行时配置

`agent-run` 为 Crush 生成的最小 provider 配置大致是：

```json
{
  "providers": {
    "deepseek": {
      "type": "openai-compat",
      "base_url": "https://api.deepseek.com",
      "api_key": "...",
      "default_large_model_id": "deepseek-v4-pro",
      "default_small_model_id": "deepseek-v4-pro",
      "models": [
        {
          "id": "deepseek-v4-pro",
          "name": "deepseek-v4-pro"
        }
      ]
    }
  }
}
```

同时会先读取已有的全局 `crush.json`，再把生成内容 merge 进去。

## 项目级行为

Crush 启动后仍会读取当前工作目录，并可能初始化项目级文件。

已确认的行为包括：

- 读取项目中的 `AGENTS.md` / `CRUSH.md` / 其他上下文文件
- 在项目初始化时创建 `AGENTS.md`

这属于 Crush 自身设计，不是 `agent-run` 的副作用。

因此 `agent-run` 对 Crush 的策略是：

- 隔离全局配置和数据目录
- 不干预 Crush 自己的项目级上下文机制

## 参数透传

Crush 适配应当把底层参数原样透传。

例如：

```bash
agent-run launch deepseek crush
agent-run launch deepseek crush run "summarize this repository"
agent-run launch deepseek crush -- run --quiet "hello"
```

最终都应当变成：

- `crush <args...>`

`agent-run` 只负责准备 provider/model/runtime config，不接管 Crush 的子命令语义。

## 风险点

- `CRUSH_GLOBAL_CONFIG` 如果错误地指向文件路径，Crush 会再拼一次 `crush.json`，导致路径错误
- Crush 仍会读取当前项目目录，不能把“隔离全局配置”误解成“隔离整个项目上下文”
- 如果某个 agent 工具做不到按进程临时隔离 provider/model，就不应该被 `agent-run` 支持

## 参考

- <https://github.com/charmbracelet/crush>
- <https://github.com/charmbracelet/crush/blob/main/README.md>
- <https://github.com/charmbracelet/crush/blob/main/AGENTS.md>
