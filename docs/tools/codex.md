# Codex

## 配置入口

`Codex` 的核心不是“环境变量直控一切”，而是：

- `config.toml`
- `model_providers`
- `profiles`
- 登录态或 API key

常见位置：

- 用户级配置：`~/.codex/config.toml`
- 项目级配置：`.codex/config.toml`
- 认证状态：`~/.codex/auth.json` 或系统 keyring

## 认证与 provider

Codex 支持两类认证路径：

- `ChatGPT` 登录
- `API key`

但如果你要接入代理、本地 provider、OpenAI-compatible 端点，更推荐走 provider/profile，而不是直接折腾登录态。

关键配置字段通常包括：

- `model_providers.<id>.base_url`
- `model_providers.<id>.env_key`
- `model_providers.<id>.wire_api`
- `profiles.<name>.model_provider`
- `profiles.<name>.model`

启动时通过 `--profile <name>` 切换，是最自然的做法。

## 配置优先级思路

Codex 官方重点强调的是 profile 和 provider 的配置方式，而不是把所有行为都塞给 shell env。

比较稳的理解是：

- 登录方式是认证层
- provider/profile 是请求路由层
- profile 比“直接改环境变量”更适合做长期维护

另外，官方支持受管环境中强制指定登录方法，例如：

- `forced_login_method = "chatgpt"`
- `forced_login_method = "api"`

这更像组织级控制，而不是日常切换手段。

## launcher 适配建议

对 Codex，优先级建议如下：

1. 优先切 `--profile`
2. 其次切 `config.toml` 上下文
3. 最后才考虑切 `auth.json`

原因：

- profile 方案稳定，兼容升级
- auth 文件是内部状态，直接写入的风险更高

适合的启动形式：

```yaml
tool: codex
args:
  - "--profile"
  - "myproxy"
```

如果你的 launcher 想统一管理 provider，可以由 launcher 生成或维护 `config.toml` 片段，再调用 `codex --profile ...`。

## 什么时候需要动 auth context

只有在下面场景，才值得把“多账号切换”做进第一版：

- 你确实要在多个 ChatGPT 登录身份之间切换
- 这些身份对应不同权限或额度
- provider/profile 不能满足需求

否则，大部分场景只靠 API provider + profile 就够了。

## 风险点

- 把“登录身份切换”和“provider 切换”混在一起，维护会很快变复杂
- 直接改 `auth.json` 要承担未来格式变化的兼容成本
- 项目级 `.codex/config.toml` 只有在 trusted project 下才会生效，launcher 要考虑工作目录上下文

## 适合的 profile 结构

```yaml
tool: codex
profile: myproxy
args:
  - "--profile"
  - "myproxy"
```

或：

```yaml
tool: codex
config:
  provider: myproxy
  model: gpt-5.3-codex
```

再由你的 launcher 渲染成 `config.toml` 与 `--profile`。

## 参考

- <https://developers.openai.com/codex/auth>
- <https://developers.openai.com/codex/config-reference>
