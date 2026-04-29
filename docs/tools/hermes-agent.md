# Hermes Agent

## 配置入口

`Hermes Agent` 的主配置目录是 `~/.hermes/`。

常见文件：

- `~/.hermes/config.yaml`
- `~/.hermes/.env`
- `~/.hermes/auth.json`

官方文档同时提到：

- 主模型的 provider / model / base URL 以 `config.yaml` 为准
- custom OpenAI-compatible endpoint 的 API key 可以通过环境变量提供

## 与 `agent-run` 的适配策略

`agent-run` 对 Hermes 先只支持：

- `openai-chat-completions`

原因：

- 官方明确支持任意 OpenAI-compatible `/v1/chat/completions`
- 官方文档说明主模型的 `base_url` 和 `model` 应该写入 `config.yaml`
- 旧的 `OPENAI_BASE_URL` / `LLM_MODEL` 主模型环境变量路径已经被移除

因此 Hermes 不适合像 `claude` 那样只靠启动时 env。

当前推荐策略是：

1. 在 `~/.cache/agent-run/hermes/<provider>/` 生成临时 `config.yaml`
2. 启动时设置 `HERMES_HOME` 指向该运行时目录
3. 启动时设置 `OPENAI_API_KEY`
4. 运行 `hermes`

## 运行时配置

`agent-run` 为 Hermes 生成的最小配置应当覆盖：

```yaml
model:
  default: your-model
  provider: custom
  base_url: https://example.com/v1
```

再通过环境变量注入：

- `OPENAI_API_KEY`

这样可以同时满足：

- 配置文件控制主模型、provider、base URL
- 密钥不写入临时配置文件

## 参数透传

Hermes 适配应当支持把 agent 参数原样透传给底层命令。

例如：

```bash
agent-run launch deepseek hermes chat -q "hello"
agent-run launch openrouter hermes -- chat --provider openrouter
```

最终都应当变成：

- `hermes <args...>`

`agent-run` 只负责准备运行时配置，不拦截 Hermes 自己的子命令和参数。

## 风险点

- Hermes 的主模型配置以 `config.yaml` 为准，不应假设 `OPENAI_BASE_URL` 足够
- 如果用户已有 `HERMES_HOME`，运行时目录和原始配置目录不能原地覆盖
- 如果 Hermes 将来再次调整 custom provider schema，运行时 `config.yaml` 模板需要跟着更新

## 参考

- <https://hermes-agent.nousresearch.com/docs/reference/environment-variables>
- <https://hermes-agent.nousresearch.com/docs/integrations/providers>
- <https://hermes-agent.nousresearch.com/docs/user-guide/configuration/>
