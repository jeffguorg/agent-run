# Crush

## 配置入口

`Crush` 的配置分成两层：

- 全局配置目录
- 当前项目目录

对 `agent-run` 最关键的是这几个入口：

- `CRUSH_GLOBAL_CONFIG`
- 当前工作目录下的项目级上下文文件

其中有一个容易踩坑的点：

- `CRUSH_GLOBAL_CONFIG` 指向的是配置目录，不是 `crush.json` 文件本身

也就是说，agent-run 通过 `CRUSH_GLOBAL_CONFIG` 定位 `crush.json` 所在目录，然后读写其中的 `crush.json`。

## 与 `agent-run` 的适配策略

`agent-run` 对 Crush 的适配目标是：

- 把 `providers.*` 同步成 Crush 可动态切换的 provider
- 保留用户手写的非 agent-run provider
- 运行时注入 secrets，不把真实 key 写入 `crush.json`
- 同步最小可用的模型元数据到 `crush.json`，让 Crush 在 provider API 不可达时仍有模型可选
- 写入 Crush `options`，让 Crush 使用同步后的 provider 集并停止自动更新 provider
- 允许沿用 Crush 自己的项目级行为

当前推荐策略是：

1. 读取全局 Crush 配置目录下的 `crush.json`
2. 写入或更新 `providers.<name>` 中由 agent-run 管理的 provider
3. 移除已经不在 `agent-run` 配置里的 stale managed provider
4. 通过 `$agent-run-managed` 判断 provider 是否由 agent-run 管理；不覆盖同名的非 agent-run provider，遇到冲突直接报错
5. 删除旧的顶层 `provider` 字段，不试图替 Crush 选择启动 provider
6. 写入 `options.disable_default_providers = true` 和 `options.disable_provider_auto_update = true`
7. 同步最小可用的模型元数据：`id` 和 `name`
8. 启动时为所有 managed provider 注入对应的 API key env var
9. 不对 `crush run` 做额外参数处理

这样做的结果是：

- Crush 可以在运行中看到并切换所有 agent-run provider
- `crush.json` 只保存 env var placeholder，不保存真实 key，并用 `$agent-run-managed` 标记托管 provider
- agent-run 不修改 Crush 当前选中的 provider 或 model
- 模型列表由 agent-run 同步最小元数据，Crush 也可自行从 provider API 发现补充
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

`agent-run` 为 Crush 生成的 provider 配置大致是：

```json
{
  "options": {
    "disable_default_providers": true,
    "disable_provider_auto_update": true
  },
  "providers": {
    "deepseek": {
      "$agent-run-managed": "deepseek",
      "type": "openai-compat",
      "base_url": "https://api.deepseek.com",
      "api_key": "${DEEPSEEK_API_KEY:?this provider is managed by agent-run, use agent-run launch deepseek crush to launch crush}",
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

`$agent-run-managed` 标记该条目由 agent-run 管理。`models` 列表只保留 Crush 实际工作需要的最小字段，Crush 也会自行从 provider API 发现补充。

同时会先读取已有的全局 `crush.json`，再只更新 agent-run 管理的 provider 条目。

## 项目级行为

Crush 启动后仍会读取当前工作目录，并可能初始化项目级文件。

已确认的行为包括：

- 读取项目中的 `AGENTS.md` / `CRUSH.md` / 其他上下文文件
- 在项目初始化时创建 `AGENTS.md`

这属于 Crush 自身设计，不是 `agent-run` 的副作用。

因此 `agent-run` 对 Crush 的策略是：

- 直接同步到全局 `crush.json`，不隔离
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
- 交互式 Crush 的 provider/model 选择不能被 `agent-run launch ... crush` 通用地强制设置
- provider name 会映射成 `<PROVIDER>_API_KEY` 环境变量，重名映射必须报错
- Crush 仍会读取当前项目目录，不能把“隔离全局配置”误解成“隔离整个项目上下文”
- 如果某个 agent 工具做不到按进程临时隔离 provider/model，就不应该被 `agent-run` 支持

## 参考

- <https://github.com/charmbracelet/crush>
- <https://github.com/charmbracelet/crush/blob/main/README.md>
- <https://github.com/charmbracelet/crush/blob/main/AGENTS.md>
