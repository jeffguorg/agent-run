# Coding Agent Launcher Notes

这份文档整理了几类常见 coding agent 在以下几个方面的配置模型：

- 配置文件放在哪里
- 凭据和登录态放在哪里
- 环境变量和配置文件谁优先
- 如何切换 provider / profile / 登录方式
- 如果要做一个类似 `ollama launch` 的启动器，应该怎么接

仓库里的文档分成两层：

- 总览文档：本文件，讲统一抽象、设计建议、工具对比
- 细节文档：`docs/tools/*.md`，分别记录每个工具的行为和注意点

## 核心结论

如果你要做一个统一 launcher，不要把抽象建成“切登录”。更稳的抽象是三步：

1. 选择配置上下文：配置目录、配置文件、profile
2. 清理冲突环境变量：尤其是已经存在的 key、token、base URL
3. 注入本次启动需要的 env / CLI 参数，再 `exec` 到真实 CLI

这三步几乎能覆盖 `Claude Code`、`Codex`、`Hermes Agent`、`OpenCode`、`Gemini CLI`、`aider` 这几类工具。

## 配置模型对比

| Tool | 主配置方式 | 凭据方式 | 优先级特征 | 更适合的 launcher 模式 |
|---|---|---|---|---|
| Claude Code | 分层 `settings.json` | 登录态 + env | 设置层级清楚，env 常常更强 | 清 env + 注入 env + 可选切 config dir |
| Codex | `config.toml` + profile | ChatGPT 登录或 API key | provider/profile 驱动很强 | 切 profile / provider，必要时切 auth context |
| Hermes Agent | `config.yaml` + `.env` | env + config | 主模型配置以 `config.yaml` 为准 | 临时 config + 临时 env |
| OpenCode | 合并式 config | auth.json 或 config | 配置文件优先于 provider env | 切 config / project context |
| Gemini CLI | settings + env + CLI flags | API key / OAuth | env 和 CLI 参数优先级明确 | 注入 env / flags 最自然 |
| aider | CLI + yaml + env + `.env` | provider key env | `.env` 与 CLI 都很好用 | 临时 env / `--set-env` / `--api-key` |

## 设计建议

### 1. 区分四种可变项

做 launcher 时，建议把这些概念拆开：

- `auth_mode`：登录态、API key、token
- `provider`：OpenAI 兼容端点、本地代理、官方端点
- `profile`：模型、provider、沙箱、审批策略等一组组合配置
- `workspace_context`：项目目录、项目级配置、trusted project 状态

如果把这些都混成“账号切换”，后面会很快失控。

### 2. 先清理，再注入

很多 CLI 都会从父进程继承环境变量。你如果只是覆盖一部分变量，常见问题有：

- 旧 token 还在，被错误复用
- 新 `base_url` 和旧 `api_key` 混用
- 配置文件里启用了某个 profile，但 shell 里有高优先级 env

所以 launcher 最稳的动作是：

1. `unset` 已知冲突变量
2. 设置当前 profile 需要的最小 env 集
3. 再启动真实 CLI

### 3. 配置目录切换通常比改登录文件更稳

如果工具本身支持配置目录切换，优先用配置目录隔离，而不是直接改登录态文件：

- `Claude Code`：可以优先看 `CLAUDE_CONFIG_DIR`
- `Codex`：优先用 `config.toml` 的 provider/profile；只有确实要切 ChatGPT 登录身份时，再考虑 auth 文件

直接写登录文件的问题是：

- 格式变更风险高
- 容易和工具内置登录流程打架
- 不同版本兼容性差

### 4. 对用户暴露“声明式 profile”，不要暴露原始 env

对外配置建议长这样：

```yaml
profiles:
  claude-proxy:
    tool: claude
    env:
      ANTHROPIC_AUTH_TOKEN: "..."
      ANTHROPIC_BASE_URL: "https://proxy.example.com"
    unset:
      - ANTHROPIC_API_KEY

  codex-local:
    tool: codex
    args: ["--profile", "ollama"]
```

这样你自己的 launcher 可以维护统一语义，底层再映射到各工具的 env / args / config。

## 推荐的目录结构

```text
docs/
  notes.md
  tools/
    claude-code.md
    codex.md
    crush.md
    hermes-agent.md
```

## 文档索引

- [Claude Code](tools/claude-code.md)
- [Codex](tools/codex.md)
- [Crush](tools/crush.md)
- [Hermes Agent](tools/hermes-agent.md)

## 哪些工具最适合先接

如果你要先做一个 MVP，建议顺序是：

1. `Claude Code`
2. `Codex`
3. `Gemini CLI`
4. `aider`
5. `OpenCode`

原因很简单：

- `Claude Code` 和 `Codex` 是你当前关注的主对象
- `Gemini CLI`、`aider` 对 env / CLI 覆盖支持很直接
- `OpenCode` 更像一个配置中心，值得接，但优先级可以稍后

## 参考文档

- Claude Code env vars: <https://code.claude.com/docs/en/env-vars>
- Claude Code configuration: <https://code.claude.com/docs/en/configuration>
- Codex auth: <https://developers.openai.com/codex/auth>
- Codex config reference: <https://developers.openai.com/codex/config-reference>
- OpenCode config: <https://opencode.ai/docs/config/>
- OpenCode providers: <https://opencode.ai/docs/providers/>
- Gemini CLI configuration: <https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md>
- aider config: <https://aider.chat/docs/config.html>
- aider dotenv: <https://aider.chat/docs/config/dotenv.html>
