# Veydeck 网站管理套件

在 Veydeck 节点上部署静态网站和反向代理，支持 HTTPS、WebSocket、证书管理、独立日志与完整备份。套件使用 Vue 3、Rust/Axum 和官方 Nginx 镜像，由管理服务和 Nginx 两个容器组成。

## 安装与构建

通过 Veydeck 应用库安装网站管理套件。每个节点独立运行套件，同一节点上的多个网站按域名和端口区分。

本地构建管理镜像：

```bash
./build-image.sh 0.1.0-alpha.1
docker compose pull nginx
```

不传版本时，从 `backend/Cargo.toml` 读取默认版本；也可通过 `RELEASE_VERSION` 设置版本、`IMAGE_REPOSITORY` 覆盖镜像仓库，通过 `CARGO_REGISTRY_MIRROR` 指定构建时的 Cargo 镜像源。前端和后端在 Dockerfile 内构建，宿主机只需安装 Docker。

在 GitHub Actions 中手动运行 **Publish Docker Image**，填写版本后发布 `veydeck/website-manager:<版本>` 并同步 Docker Hub README。仓库需配置 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN` 两个 Actions Secrets。该工作流仅发布管理镜像，Nginx 继续使用官方镜像。

套件清单位于 `veydeck-suites` 仓库的 `suites/veydeck.websites`，在该仓库执行以下命令生成安装包：

```bash
bash scripts/package.sh suites/veydeck.websites
```

管理入口通过 Veydeck 可信代理访问。管理服务通过 Agent 提供的固定操作检查和重载 Nginx 配置，Nginx 使用官方镜像并固定摘要。

## 网站管理

- 支持静态网站和 HTTP/HTTPS 反向代理，代理支持 WebSocket。
- 创建网站时必须填写域名或 IP；第一项为主域名。同一端口内域名不能重复，不同端口可以复用。
- 填写 `0.0.0.0` 可接收对应端口的所有域名请求。网站默认监听全部 IPv4 网卡，可选择同时监听 IPv6。
- 网站代号默认取第一个域名，也可自行填写；代号在节点内唯一，创建后固定。
- 创建、保存、启停和删除均检查配置并自动重载；配置检查或重载失败时保留原配置。
- 列表展示名称、类型、协议、状态、网站目录和操作。配置证书的网站显示 HTTPS，否则显示 HTTP。

网站目录固定为 `/opt/veydeck/data/www/websites/sites/<代号>/index`。创建静态网站时自动生成 `index.html` 和 `404.html`，运行目录默认是网站目录，也可设置为 `dist` 等相对子目录。

点击网站目录图标，可在对应节点的文件管理应用中上传文件或目录。当前运行目录内的文件修改直接生效；修改运行目录后点击“保存并重载”。删除网站默认保留文件，也可选择同时永久删除目录。

## 证书管理

“证书”分组包含证书列表、自签证书和上传证书。

上传证书 PEM 和私钥后，自动解析域名、颁发组织、有效期、指纹、公钥与证书链等信息。证书时间使用浏览器本地时区显示，可在网站设置中选择证书以启用 HTTPS。

每个套件实例独立生成名为 `Veydeck` 的内置根证书，到期时间为 `9999-12-31 23:59:59 UTC`。支持创建自定义根证书，密钥算法可选 EC 256、RSA 2048 和 RSA 4096。

选择根证书可为域名、通配域名、IPv4 或 IPv6 签发网站证书。有效期以年配置，默认为 10 年，范围为 1–99 年，实际到期时间不超过根证书。下载根证书仅提供公开 PEM；客户端安装并信任对应根证书后，才能信任其签发的证书。

## 设置

- **性能**：调整 Nginx 全局参数，包括域名哈希桶、请求头缓冲区、请求体大小、长连接超时，以及 gzip 开关、最小压缩大小和压缩级别。保存并检查通过后自动重载，网站显式配置的同名指令按 Nginx 继承规则生效。
- **日志**：查看 Nginx 运行错误及未匹配网站的访问日志。单个网站的访问和错误日志在网站详情中查看，切换类型时自动刷新。
- **备份**：下载网站文件、配置、数据库及证书材料的完整归档，包含网站私钥和根证书私钥。

网站、证书列表和自签证书采用固定区域分页，每页条数随窗口高度自动调整，底部分页器显示总条数及当前页。

## 数据存储

数据默认保存在 `/opt/veydeck/data/www/websites`：

```text
sites/<代号>/index/                  网站目录
instances/<实例ID>/state/            数据库
instances/<实例ID>/certificates/     网站证书与私钥
instances/<实例ID>/authorities/      根证书与私钥
instances/<实例ID>/current/          当前 Nginx 配置
instances/<实例ID>/logs/nginx/       Nginx 日志
instances/<实例ID>/logs/sites/<代号>/ 网站日志
```

全局配置为 `current/nginx.conf`，按网站分别加载 `current/conf.d/<代号>.conf`。Nginx 和网站日志独立保存，每日轮转并保留七天。

管理服务重启时，Nginx 可继续提供网站服务。网站配置、性能参数与证书材料持久保存，升级前可通过“设置 → 备份”下载完整归档。

## 语言

支持简体中文和英文，嵌入主控时跟随主控语言实时切换，独立打开时读取浏览器语言。切换语言保留当前输入和分页位置，日期、数量与业务错误同步本地化。用户输入、证书事实及原始技术日志保持原文。

界面翻译位于 `frontend/src/locales/messages.json`，业务错误翻译位于 `frontend/src/locales/errors.json`。

## 版本记录

参见 [CHANGELOG.md](CHANGELOG.md)。
