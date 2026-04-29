# Claude Code

## 配置入口

`Claude Code` 的配置主要分三类：

- 分层 `settings.json`
- shell 环境变量
- 登录态和配置目录

常见位置：

- 用户级：`~/.claude/settings.json`
- 项目级：`.claude/settings.json`
- 项目本地覆盖：`.claude/settings.local.json`
- 登录态与部分本地状态：`~/.claude.json`

官方还提供：

- `CLAUDE_CONFIG_DIR`：切换整套 Claude 配置目录

## 配置优先级

官方明确给了 `settings.json` 体系内的优先级：

`Managed` > `命令行参数` > `.claude/settings.local.json` > `.claude/settings.json` > `~/.claude/settings.json`

环境变量是另一条配置通道。官方没有给出“shell env 与 `settings.json.env` 同名冲突时”的总优先级表，但已经明确说明，多个具体 env 会覆盖设置项，例如：

- `CLAUDE_CODE_EFFORT_LEVEL` 会覆盖 `/effort` 和 `effortLevel`
- `CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS` 会覆盖 `includeGitInstructions`
- `ANTHROPIC_API_KEY` 会覆盖已登录的 Claude 订阅身份

工程上应当按下面的规则处理：

- 不假设 shell env 与 `settings.json.env` 的合并顺序
- 如果 launcher 要强制指定 provider，就先 `unset` 相关变量，再显式 `export`

## provider / auth 切换

Claude 这边最实用的切换手段是 env：

- `ANTHROPIC_API_KEY`
- `ANTHROPIC_AUTH_TOKEN`
- `ANTHROPIC_BASE_URL`

典型用途：

- 切到官方 API key
- 切到代理或 OpenAI-compatible 网关
- 临时绕过默认登录态

如果你需要彻底隔离不同上下文，可以配合 `CLAUDE_CONFIG_DIR` 使用多套配置目录。

## launcher 适配建议

最稳的启动流程：

1. 清理继承到的 `ANTHROPIC_*` 变量
2. 视需要设置 `CLAUDE_CONFIG_DIR`
3. 注入当前 profile 的 token / key / base URL
4. `exec claude "$@"`

建议至少清理这些变量：

- `ANTHROPIC_API_KEY`
- `ANTHROPIC_AUTH_TOKEN`
- `ANTHROPIC_BASE_URL`

## 风险点

- 如果你只覆盖 `ANTHROPIC_BASE_URL`，但没有清掉旧 `ANTHROPIC_API_KEY`，很容易出现错配
- 如果项目里有 `.claude/settings.local.json`，它可能让同一套启动参数在不同目录下行为不同
- 直接依赖 `~/.claude.json` 的内部格式风险较高，不适合做第一版方案

## 适合的 profile 结构

适合用“env launcher”来表达：

```yaml
tool: claude
unset:
  - ANTHROPIC_API_KEY
  - ANTHROPIC_AUTH_TOKEN
  - ANTHROPIC_BASE_URL
env:
  ANTHROPIC_AUTH_TOKEN: "..."
  ANTHROPIC_BASE_URL: "https://proxy.example.com"
args:
  - "--model"
  - "claude-sonnet-4-5"
```

## 参考

- <https://code.claude.com/docs/en/env-vars>
- <https://code.claude.com/docs/en/configuration>
- <https://code.claude.com/docs/en/authentication>
