# GitHub 发布与更新

CI 在推送或提交 PR 到 `main` 时检查桌面端、Agent、终端运行时、更新逻辑和打包脚本。

`Release GPUI Desktop` 从 `main` 可达的版本标签构建 Windows x64、macOS Apple Silicon / Intel、各平台 Agent 和 WSL 运行时。发布前，在中英文 `CHANGELOG` 中写入与标签一致的版本节。推送标签即可启动流程，也可在 Actions 中选择已有标签重跑。

安装包和 Agent 产物全部上传后，发布流程将清单写入 `main` 的 `updates/stable/latest.json` 或 `updates/beta/latest.json`。正式版也可推进 beta 清单，各渠道保留版本较新的清单。首次发布前，清单显示该渠道暂无发布。

默认发布提供浏览器下载安装包。若启用应用内验签安装，在 GitHub Actions 配置本仓库的 `CODUX_UPDATER_PUBLIC_KEY` 仓库变量和与之匹配的 `TAURI_SIGNING_PRIVATE_KEY` Secret。密钥有密码时，同时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。客户端编译时内置公钥，已有未内置该公钥的版本可用浏览器下载安装包。

`Release macOS Formal` 可手动为已有 Release 补充公证后的 macOS 产物。
