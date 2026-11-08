# TackNote

开源的三端桌面便签,支持 macOS、Windows 和安卓。便签通过你自己的 WebDAV 空间同步,不需要注册账号,也没有官方服务器。

An open-source sticky notes app for macOS, Windows and Android. Notes sync through your own WebDAV space, with no account and no vendor server.

> 状态:第一个版本 v0.1.0。Windows 版目前只经过自动测试。 / Status: first release, v0.1.0. The Windows build has only passed automated tests so far.

## 功能 / Features

- 彩色便签、置顶、搜索、拖动排序
- 待办:在编辑页把一行设为待办,点前面的方框勾选
- 电脑上可以把便签贴到桌面,变成一个可以保持在最前面的小窗口
- 安卓桌面小组件:每个小组件显示一张便签,可以直接勾选待办
- 通过 WebDAV 在多台设备间同步,排序也会同步
- 两台设备同时修改同一张便签时,两边的文字都会保留
- 中文和英文界面,浅色和深色主题

Coloured notes, pinning, search and drag to reorder; to-do lines with a box to tick; notes stuck on the desktop as small windows that can stay on top; Android home screen widgets, one note each, with ticking; sync over WebDAV, order included; both texts kept when two devices change the same note; Chinese and English, light and dark.

## 下载和安装 / Install

从 [Releases](../../releases) 页面下载对应的文件。应用没有购买开发者证书,所以第一次打开时系统会提示风险,这是正常的。

Download from the [Releases](../../releases) page. The app is not signed with a paid developer certificate, so each system warns on first launch; this is expected.

**macOS**(`.dmg`,同时支持 Apple 芯片和 Intel)

1. 打开 `.dmg`,把 TackNote 拖进“应用程序”。
2. 第一次打开时,系统会说无法验证开发者。先点“完成”,再打开“系统设置 → 隐私与安全性”,在页面下方点“仍要打开”。
3. 第一次同步时,系统可能询问是否允许 TackNote 使用钥匙串里的密码,选“始终允许”。

Open the `.dmg` and drag TackNote into Applications. On first launch macOS says it cannot verify the developer: close the message, then go to System Settings → Privacy & Security and click Open Anyway. If asked whether TackNote may use the password in your keychain, choose Always Allow.

**Windows**(`-setup.exe`)

运行安装程序时如果出现“Windows 已保护你的电脑”,点“更多信息”,再点“仍要运行”。

If “Windows protected your PC” appears, click More info, then Run anyway.

**安卓**(`.apk`,64 位 ARM 手机)

在手机上打开 `.apk`,按提示允许“安装未知应用”。添加桌面小组件:长按桌面空白处,在小组件里找到 TackNote,再选要显示的便签。

Open the `.apk` on the phone and allow installing unknown apps when asked. To add a widget, touch and hold an empty spot on the home screen, pick TackNote from the widgets, then choose a note.

## 设置同步 / Set up sync

在设置页填 WebDAV 地址、用户名和密码,点“保存并同步”。每台设备填同一个账号即可。密码只保存在系统的安全存储里(macOS 钥匙串、Windows 凭据管理器、安卓 Keystore),不会写进普通文件。

In Settings, enter the WebDAV address, user name and password, then Save and sync. Use the same account on every device. The password is kept only in the system's secure storage (macOS Keychain, Windows Credential Manager, Android Keystore).

**坚果云 / Jianguoyun**

- 地址 / Address:`https://dav.jianguoyun.com/dav/`
- 用户名:你的坚果云账号
- 密码:不是登录密码,而是“应用密码”。在坚果云网页上点账户名 → 账户信息 → 安全选项 → 第三方应用管理 → 添加应用,生成一个。

**自己的 WebDAV 服务器 / Your own WebDAV server**

填服务器给你的 WebDAV 地址即可,请使用 `https` 地址。已经实测过坚果云和基于 SFTPGo 的服务器。

Enter the WebDAV address your server gives you; use `https`. Tested with Jianguoyun and an SFTPGo-based server.

## 同步是怎么工作的 / How sync works

所有便签保存在 WebDAV 的 `tacknote/notes.json` 文件里,每张便签单独合并:

- 只有一边改过:用改过的版本。
- 两边都改了文字:较新的版本保留原便签,较旧的版本另存为“冲突副本”。
- 一边删除、另一边修改:保留修改,不丢文字。
- 服务器上的文件损坏,或来自更新的版本:停止同步,不会覆盖它。

有的服务器(比如 SFTPGo)不会拦住“基于旧版本的上传”,两台设备恰好同时同步时,后写的会覆盖先写的。每张便签都记着每台设备最后一次改它的时间,被覆盖的设备下次同步时会认出来,把自己的内容放回去,所以文字不会丢。

Each note is merged on its own: a change on one side wins; text changed on both sides keeps the newer edit and saves the older one as a conflict copy; an edit beats a deletion; a damaged file or one from a newer version stops sync instead of being overwritten. Some servers do not refuse an upload based on an old version, so two devices syncing at the same moment can overwrite each other; every note records when each device last edited it, so the overwritten device notices on its next sync and puts its text back.

删除记录保留 90 天。离线超过 90 天的设备重新同步时,已删除的便签可能会回来。

数据以明文保存在你的 WebDAV 空间里,请使用 https 地址和你信任的服务。

Deletions are remembered for 90 days; a device offline for longer may bring a deleted note back. Notes are stored as plain text in your WebDAV space, so use https and a service you trust.

## 开发 / Development

使用 [Tauri 2](https://tauri.app/)。需要 Rust、Node.js 和对应平台的工具:macOS 只需要 Xcode Command Line Tools,安卓需要 Android SDK、NDK 和 JDK 17。

Built with [Tauri 2](https://tauri.app/). You need Rust, Node.js and the platform tools: on macOS only the Xcode Command Line Tools, on Android the Android SDK, NDK and JDK 17.

```bash
npm install
npm test
cd src-tauri && cargo test --lib && cd ..
npm run tauri dev
```

发布:把 `src-tauri/tauri.conf.json` 里的版本号改好,推送同名的标签(比如 `v0.1.0`),GitHub Actions 会打包三个平台并建一个草稿版本。安卓的正式签名密钥放在仓库的 Secrets 里,见 `.github/workflows/release.yml` 开头的说明。

Releasing: set the version in `src-tauri/tauri.conf.json` and push a matching tag such as `v0.1.0`; GitHub Actions builds all three platforms and makes a draft release. The Android release key lives in the repository secrets, described at the top of `.github/workflows/release.yml`.

## 许可证 / License

[MIT](LICENSE)
