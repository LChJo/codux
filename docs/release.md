# GitHub 发布与更新

CI 在推送或提交 PR 到 `main` 时检查桌面端、Agent、终端运行时、更新逻辑和打包脚本。

`Build Desktop Test` 在打包脚本、相关工作流或 Windows 安装包资源变更推送到 `main` 后自动运行，也可手动触发。它构建 Windows x64 安装包和 macOS Apple Silicon DMG，并将产物保存在该次 Actions 运行中。

`Release GPUI Desktop` 从 `main` 可达的版本标签构建 Windows x64、macOS Apple Silicon / Intel、各平台 Agent 和 WSL 运行时。发布前，在中英文 `CHANGELOG` 中写入与标签一致的版本节。推送标签即可启动流程，也可在 Actions 中选择已有标签重跑。

安装包和 Agent 产物全部上传后，发布流程将清单写入 `main` 的 `updates/stable/latest.json` 或 `updates/beta/latest.json`。正式版也可推进 beta 清单，各渠道保留版本较新的清单。首次发布前，清单显示该渠道暂无发布。

首次发布后，应从 Releases 确认对应标签的产物已可下载，再核对 `main` 上的渠道清单与该标签一致。客户端可在 **设置 → 更新** 中选择稳定版或 Beta，点击「检查更新」验证当前平台的下载入口。

默认发布提供浏览器下载安装包。若启用应用内验签安装，在 GitHub Actions 配置本仓库的 `CODUX_UPDATER_PUBLIC_KEY` 仓库变量和与之匹配的 `TAURI_SIGNING_PRIVATE_KEY` Secret。密钥有密码时，同时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。客户端编译时内置公钥，已有未内置该公钥的版本可用浏览器下载安装包。

`Release macOS Formal` 可手动为已有 Release 补充公证后的 macOS 产物。
