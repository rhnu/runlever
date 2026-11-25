# Runlever

Runlever 在一台 Linux 主机上运行管理员预先定义的 Action。通过中文网页选择枚举参数并执行，也可以给单个 Action 开启带独立 Token 的 webhook。网页提供实时输出和可筛选的执行记录。

## 安装

从 GitHub Releases 下载与 `uname -m` 相符的发行包。以 x86_64、部署地址 `https://ops.example.com` 为例，在 VPS 上运行以下命令。ARM64 使用文件名中的 `aarch64`。

```sh
tar -xzf runlever-0.4.0-linux-x86_64.tar.gz
cd runlever-0.4.0-linux-x86_64
sudo ./runlever service install --origin https://ops.example.com
```

安装器使用 systemd 和系统账户管理工具，首次安装会生成网页登录 Token 并输出到当前终端。管理员也可从 `/etc/runlever/config.toml` 读取该 Token。示例 webhook 使用另一个随机 Token。配置由 root 拥有，`runlever` 组可读，运行记录保存在 `/var/lib/runlever/runs.sqlite3`。

将 HTTPS 反向代理指向 `127.0.0.1:7380`。例如 Caddy 的站点配置如下，域名须与 `server.origin` 一致。

```caddyfile
ops.example.com {
    reverse_proxy 127.0.0.1:7380
}
```

打开站点并用管理员 Token 登录。网页会随设备的系统设置自动切换浅色与深色主题。若代理会缓冲流式响应，请为 `/api/runs/` 关闭响应缓冲，以便日志及时显示。

升级时解压新版本并再次运行 `sudo ./runlever service install`。安装器更新二进制和 [systemd unit](packaging/runlever.service)，保留现有配置、脚本和数据库，随后重启服务。请在执行和排队的动作均已结束后升级。0.2.0 启动时会迁移数据库，保留已有执行记录和日志。

## 定义动作

编辑 `/etc/runlever/config.toml`。完整的 [示例配置](examples/config.toml) 给出了 Compose 管理、自选项目重启、固定重启 Forgejo 和单行命令四种用法。配套的 [脚本](examples/compose.sh) 安装在 `/usr/local/lib/runlever/compose.sh`。示例中的 `/opt/forgejo` 和 `/opt/immich` 应替换为本机实际的项目目录。

- `label` 是 Action 和枚举选项在网页上的名称。枚举项由管理员显式声明，`value` 是实际交给进程的值。客户端只提交选项 ID。
- `inputs = { project = "projects" }` 让用户从 `enums.projects` 中选择。也可写 `project = { enum = "projects", label = "项目" }` 自定义控件名称。输入和选项按声明顺序展示。
- `exec.program` 直接启动程序，`exec.args` 中每个固定字符串或 `{ param = "project" }` 都是一个独立参数。脚本的位置参数从 `$1` 开始。`exec.cwd` 可为固定目录或参数引用。
- `exec.command` 接受管理员编写的固定 Shell 命令。例如 `exec = { command = 'docker compose --project-directory "$1" restart', args = [{ param = "project" }] }`。执行器调用 `/bin/sh -c`，通过独立的位置参数传值。
- `call` 引用另一个 Action。`bind` 以选项 ID 固定参数。层层调用后，界面仅显示尚未绑定的输入，所有输入均已绑定时单击按钮即可执行。
- `lock = { param = "project" }` 按枚举和选项 ID 互斥。引用同一枚举同一选项的动作共享锁，派生 Action 也参与互斥。

Action、组、枚举、选项和参数的 ID 使用字母、数字、下划线或连字符，首字符为字母、数字或下划线。修改后执行以下命令使配置生效。

```sh
sudo /usr/local/bin/runlever check --config /etc/runlever/config.toml
sudo systemctl restart runlever
```

服务以专用账户 `runlever` 的身份执行命令。按实际动作配置目录读取权限和工具访问权限。如果主机通过 `docker` 组管理 Docker socket，管理员可执行 `sudo usermod -aG docker runlever` 后重启 Runlever 服务。能够访问常规 rootful Docker socket 的账户具有主机 root 级能力。[Docker 主机权限说明](https://docs.docker.com/engine/install/linux-postinstall/)

### 分组、并发和排队

```toml
[groups.compose]
label = "Compose 管理"
max_concurrent = 2

[queue]
max_wait_seconds = 60
max_pending = 16
```

在现有 Action 的配置表中设置 `group = "compose"`，网页会按组展示。组内并发上限为 `1` 时全组串行，设为 `2` 时可同时执行两条。省略 `max_concurrent` 时只按组展示。派生 Action 继承目标的组，如果目标未分组，则可单独指定组。

Action 必须同时满足组并发上限和参数锁条件才能启动。例如 Compose 组允许两条并行时，两个不同项目可以同时操作，同一项目仍然互斥。暂时无法执行的请求进入队列，服务按受理顺序启动当前满足条件的请求。

`max_wait_seconds` 是等待启动的时限，默认 60 秒，须为正整数。`max_pending` 是全服务可等待的请求数，默认 16，运行中的请求不计入。设为 0 可关闭排队。等待超时后，记录显示「排队超时」，命令不会启动。

## Webhook

每个需要从外部调用的 Action 单独添加 `webhook = { token = "独立的随机密钥" }`。各 hook 使用独立密钥，派生动作不会继承底层动作的 webhook。未启用的地址返回 `404`，认证失败返回 `401`。请求使用 `Authorization: Bearer`。

```sh
curl --fail-with-body -X POST https://ops.example.com/hooks/restart-forgejo \
  -H "Authorization: Bearer $RUNLEVER_HOOK_TOKEN" \
  -H 'Content-Type: application/json' -d '{}'
```

无参数动作也接受空正文。对于仍有枚举输入的动作，正文直接提交键值对象，例如 `{"project":"immich"}`。成功受理返回 `202` 和 `{"run_id":123}`，排队中的请求也有执行记录。队列满时，需要等待的新请求返回 `429`。未知选项、额外字段或覆盖绑定值会被拒绝。管理员可在网页查询网页及 webhook 触发的运行。

## 执行记录与日志保留

在动作所在行选择参数并点击“运行”，请求受理后，页面底部会展开此次执行的日志。收起抽屉后，命令继续执行。可从动作行重新查看最近一次执行，也可在“执行记录”按动作、状态和来源筛选，打开任意一次运行。

抽屉实时显示排队和运行状态，按序接收 stdout、stderr。向上翻阅会暂停跟随，打开“跟随输出”可回到最新内容。刷新页面可以回到同一次运行。进程输出收齐后记录退出结果，零退出码标为成功。服务重启时会将此前排队中的和运行中的请求标为中断。

`GET /api/runs/{id}` 中，`submitted_at` 是受理时间，命令尚未启动时 `started_at` 为 `null`。排队和排队超时分别使用 `queued` 和 `queue_timeout` 状态。事件流以 `status` 传送进行中的状态，以 `done` 传送最终结果。

```toml
[logs]
retention_days = 30
request_parameters = true
```

保留天数为正整数，默认 30。服务启动及其后每小时按**结束时间**清理到期的输出。活跃任务继续保留输出，运行摘要与退出结果一直可查。没有输出和输出已清理会分别显示。SQLite 会复用删除正文后释放的数据页。

使用 `systemctl status runlever` 查看系统服务，使用 `journalctl -u runlever` 查看运行器的诊断信息。`request_parameters` 控制是否在服务日志中记录通过校验的请求选项 ID，默认开启。网页的执行记录始终保留选项 ID。程序会在写入输出前遮盖已配置的登录和 webhook Token。

## 从源码构建

准备稳定版 Rust、C 编译器及满足 `web/package.json` 引擎要求的 Node.js。先构建 React 前端，再编译 Rust 二进制。SQLite 和前端资源随二进制分发。

```sh
npm --prefix web ci
npm --prefix web run build
cargo build --release --locked
```

推送 Git 标签后，GitHub Actions 会构建 Linux x86_64 和 ARM64 发行包并发布到对应的 GitHub Release。发行包文件名中的版本取自 `Cargo.toml`。

本地前台运行使用 `runlever serve --config <配置文件> --state-dir <数据目录>`。`check` 可以验证示例结构。实际启动前需要替换示例 Token 并配置外部 HTTPS 地址。

配置、绑定与调用解析在 [config](src/config.rs)，请求处理在 [http](src/http.rs)，运行与日志保存在 [engine](src/engine.rs)，界面入口在 [React 应用](web/src/App.tsx)。

## 许可证

Runlever 采用 [GNU GPL 第 3 版](LICENSE)（`GPL-3.0-only`）。发行包附带许可证全文。
