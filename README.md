# TackNote

开源的三端桌面便签,支持 macOS、Windows 和安卓。便签通过你自己的 WebDAV 空间同步,不需要注册账号,也没有官方服务器。

An open-source sticky notes app for macOS, Windows and Android. Notes sync through your own WebDAV space, with no account and no vendor server.

> 状态:开发中,还没有正式版本。 / Status: in development, no release yet.

## 功能 / Features

- 彩色便签、置顶、搜索和待办勾选(在行首写 `[ ]`)
- 通过 WebDAV 在多台设备间同步
- 两台设备同时修改同一张便签时,两边的内容都会保留
- 中文和英文界面

开发中:桌面悬浮便签窗口、安卓桌面小组件。

## 同步是怎么工作的 / How sync works

所有便签保存在 WebDAV 的 `tacknote/notes.json` 文件里,每张便签单独合并:

- 只有一边改过:用改过的版本。
- 两边都改了文字:较新的版本保留原便签,较旧的版本另存为“冲突副本”。
- 一边删除、另一边修改:保留修改,不丢文字。
- 服务器上的文件损坏,或来自更新的版本:停止同步,不会覆盖它。

删除记录保留 90 天。离线超过 90 天的设备重新同步时,已删除的便签可能会回来。

数据以明文保存在你的 WebDAV 空间里,请使用 https 地址和你信任的服务。

## 开发 / Development

使用 [Tauri 2](https://tauri.app/)。需要 Rust、Node.js 和对应平台的工具:macOS 只需要 Xcode Command Line Tools,安卓需要 Android SDK 和 NDK。

Built with [Tauri 2](https://tauri.app/). You need Rust, Node.js and the platform tools: on macOS only the Xcode Command Line Tools, on Android the Android SDK and NDK.

```bash
npm install
cd src-tauri && cargo test && cd ..
npm run tauri dev
```

## 许可证 / License

[MIT](LICENSE)
