# Terracotta | 陶瓦联机

Terracotta 为游玩 Minecraft: Java Edition 的玩家提供开箱即用的联机功能。

项目基于 EasyTier 开发，针对 Minecraft 做了大量优化，尽量降低操作门槛并集成了 HMCL 启动器支持。

## 下载

可在 [Releases](https://github.com/burningtnt/Terracotta/releases) 页面下载对应平台的发行包。

如下载缓慢，可尝试使用[国内镜像](https://gitee.com/burningtnt/Terracotta/releases)

## 作为库使用

Terracotta 也可以作为 Rust 库集成到您的项目中。本仓库维护了一个支持支持Rust stable 版本并可作为库使用的分支。

### 添加依赖

在您的 `Cargo.toml` 中添加以下依赖：

```toml
[dependencies]
terracotta = { git = "https://github.com/PCL-Community/Terracotta-lib.git", version = "2.5.0-pcl.proto" }
```

### 初始化

在使用 Terracotta 功能之前，需要先初始化库。您需要提供一个机器标识文件的路径。在 Tauri 中，你可以在 Tauri Builder 中调用下方代码初始化库：

```rust
terracotta::init_lib("/path/to/machine-id".into());
```

### 示例代码

以下是一个简单的与 Tauri 集成的示例。

```rust
use terracotta::{controller, rooms::Room};

#[tauri::command]
pub fn get_terracotta_state() -> serde_json::Value {
    controller::get_state()
}

#[tauri::command]
pub fn set_terracotta_waiting() {
    controller::set_waiting()
}

#[tauri::command]
pub fn set_terracotta_host_scanning() {
    controller::set_scanning_only();
}

#[tauri::command]
pub fn set_terracotta_guesting(room_code: String, player: String) -> Result<(), String> {
    let room = Room::from(&room_code).ok_or("invalid room code")?;
    if controller::set_guesting(room, Some(player)) {
        Ok(())
    } else {
        Err("set guesting failed".to_string())
    }
}
```

### API 参考

主要模块和函数：

- `terracotta::init_lib(path: PathBuf)` - 初始化库
- `terracotta::controller::get_state() -> serde_json::Value` - 获取当前状态
- `terracotta::controller::set_waiting()` - 设置为等待状态
- `terracotta::controller::set_scanning(room: Option<Room>, player: Option<String>)` - 开始主机扫描
- `terracotta::controller::set_guesting(room: Room, player: Option<String>) -> bool` - 加入房间作为客人
- `terracotta::rooms::Room` - 房间相关功能

## 平台特定要求

- Windows: You need to download and configure the [Npcap SDK](https://npcap.com/#download) as LIB environment variable.
- macOS: make sure that Xcode is installed. You also need to install protobuf via Homebrew (`brew install protobuf`).
- Ubuntu: you need to install the `protobuf-compiler` package.

## 许可
本程序使用 [GNU Affero General Public License v3.0 or later](https://github.com/burningtnt/Terracotta/blob/master/LICENSE) 许可，并附有以下例外：

> **AGPL 的例外情况:**
>
> 作为特例，如果您的程序通过以下方式利用本作品，则相应的行为不会导致您的作品被 AGPL 协议涵盖。
> 1. 您的程序通过打包的方式包含本作品未经修改的二进制形式，而没有静态或动态地链接到本作品；或
> 2. 您的程序通过本作品提供的进程间通信接口（如 HTTP API）与未经修改的本作品应用程序进行交互，且在您的程序用户界面明显处标识了本作品的版权信息。