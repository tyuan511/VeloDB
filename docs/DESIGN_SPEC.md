# VeloDB — 产品功能规格与 Rust + GPUI 架构设计（v0.2）

> **定位**：基于 Rust + GPUI 构建的 macOS 原生毛玻璃风格（Vibrant Native）PostgreSQL 专业工作台。对标 TablePlus 的极速直观、DataGrip 的工程深度与 Navicat 的完备管理能力；**不内置臃肿的 AI 聊天面板，而是原生提供本地 MCP Server，让用户直接接入自己的 Agent 工作流。**

---

## 1. 产品核心理念

1. **极致性能与原生质感**：GPUI GPU 加速渲染，冷启动 < 150ms，百万行结果集虚拟滚动 120fps，原生 macOS 毛玻璃层级。
2. **PostgreSQL 深度专精**：完整覆盖 Schema、Tables、Views、Materialized Views、Functions、Sequences、Enums、Extensions，以及 `JSONB`、`Array`、`UUID`、`Timestamptz`、`EXPLAIN ANALYZE`、`pg_stat_activity` 等 PG 专属特性。
3. **安全感优先（Safe Mode）**：数据与结构变更默认进入**暂存区（Staged Changes）**，类似 Git Diff 高亮展示，支持预览完整事务 SQL 后再提交；生产环境（Prod）强制保护与只读锁支持。
4. **Agent-Ready（内置 MCP Server）**：客户端本地内置轻量 MCP Server，默认只读安全策略（可按连接授权），一键复制配置给用户的外部 AI Agent（如 Claude Code、Cursor、Antigravity 等），安全复用当前连接与 Schema 上下文。

---

## 2. 完整功能模块规划

### 2.1 连接管理与环境隔离（Connection Manager）
- **环境色彩标识**：`LOCAL`（绿）、`STAGING`（琥珀）、`PRODUCTION`（红），顶部标题栏实时透出环境色。
- **完整连接配置**：
  - 支持标准表单与 **PostgreSQL URI 一键粘贴解析**。
  - **SSL/TLS 模式**：`disable` / `prefer` / `require` / `verify-full`。
  - **SSH 跳板机隧道**：支持密码或私钥（`~/.ssh/id_ed25519`）认证。
  - **安全选项**：只读模式（Read-Only Lock）、Safe Mode 默认开启。
  - **连接测试**：实时返回服务端版本与 RTT 延迟。

### 2.2 Schema 对象浏览器（Left Vibrant Sidebar）
- **多 Schema 切换**：快速切换 `public`、`analytics`、`auth` 等 Schema。
- **完整 PG 对象树**：可折叠分组展示 **Favorites（收藏）**、**Tables**（带估算行数与表体积）、**Views / Materialized Views**、**Functions**、**Sequences**、**Enums / Types**、**Extensions**。
- **实时模糊过滤 + 右键菜单**：右键支持 Open Data、Open Structure、View DDL、Copy Name、Export、Pin to Favorites。

### 2.3 专业数据网格（Data Grid）
- **高性能浏览**：虚拟滚动、列宽调整、**点击列头排序（`ASC` / `DESC`）**、**列显示/隐藏选择器（Columns Picker）**、分页与 `LIMIT` 切换。
- **类型感知渲染与外键跳转**：
  - `NULL` 斜体灰显、`JSONB` 胶囊摘要、`ENUM` 状态徽章、`UUID` 等宽展示。
  - **Foreign Key Jump**：外键列（如 `orders.user_id`）带跳转图标，一键定位到目标表（`users`）对应记录。
- **单元格右键菜单与快捷操作**：Copy Value、Copy Row as JSON / SQL INSERT、Set NULL、Clone Row、Delete Row。
- **暂存修改（Staged Changes）**：行内双击编辑（琥珀色高亮 + 脏角标）、新增行（绿）、删除行（红），底部悬浮栏提供 `Preview SQL`、`Discard`、`Commit`。

### 2.4 表结构设计器（Structure & DDL）
在表标签页内一键切换至 `Structure`，包含 4 个专业子视图：
- **Columns**：字段名、类型、PK/FK 标识、Nullable、Default、Comment，支持新增列并生成 `ALTER TABLE` 预览。
- **Indexes**：索引名称、字段、方法（`btree` / `gin` / `hnsw`）、Unique、体积与扫描次数统计。
- **Constraints & Foreign Keys**：主键、外键关联目标及 `ON DELETE / ON UPDATE` 策略、Check 约束。
- **DDL**：完整 `CREATE TABLE` + 索引 + 约束的完整 SQL 定义，一键复制。

### 2.5 SQL 工作台（SQL Workbench）
- **专业编辑体验**：带行号编辑器、**SQL 格式化（Format）**、`⌘⏎` 执行、支持多预设与自定义 **Snippets（运维/业务代码片段）**。
- **多维度结果面板**：
  - `Results`：结果集表格，支持直接导出。
  - `EXPLAIN ANALYZE`：树形可视化执行计划，展示算子耗时占比、Actual vs Planned Rows、Buffer 命中与慢查询索引建议。
  - `Messages`：执行日志、耗时、受影响行数与报错定位。
  - `History` & `Snippets`：可搜索的查询历史与内置 PG 常用诊断 SQL（查锁、查慢查询、查表大小）。

### 2.6 数据库活动与健康监控（Server Monitor）
- **概览指标卡**：活跃连接数、Buffer 缓存命中率（Cache Hit Ratio）、数据库总体积、TPS / 延迟。
- **Active Queries (`pg_stat_activity`)**：实时查看当前会话 PID、用户、状态（`active` / `idle in transaction` / `waiting`）、耗时与 SQL，支持一键 **Cancel** 或 **Terminate**。
- **Storage & Table Sizes**：展示各表 Data + Index + TOAST 占用排行，辅助发现表膨胀。

### 2.7 右侧双模式检视器（Cell & Row Inspector）
- **Cell Inspector**：针对选中单元格深度展示与编辑——`JSONB` 支持 Pretty Format 与校验、`Timestamptz` 自动展示 UTC + 本地时区换算、`UUID`/长文本便捷编辑并一键回写暂存区。
- **Row Inspector**：将当前选中整行以**竖排表单（Key-Value）**呈现，宽表（30+ 列）场景下无需横向滚动即可快速浏览和编辑字段。

### 2.8 导入导出 & 全局 `⌘K` & 本地 MCP Server
- **Import / Export**：支持导出为 `CSV` / `JSON` / `SQL INSERT`，以及拖拽导入 CSV/JSON 并预览列映射。
- **Command Palette（`⌘K`）**：全局跳转任意表、打开 Server Monitor、切换连接、新建 SQL、切换主题与安全模式。
- **内置 MCP Server（替代 AI Copilot）**：顶部状态栏提供 `MCP` 开关与配置弹窗，支持开关本地 stdio/SSE MCP 端点、勾选只读保护（Read-Only for Agents）、暴露 `list_schemas` / `describe_table` / `run_read_query` / `explain_query` 等工具，一键复制配置到用户的 Agent。

### 2.9 设置中心（Settings `⌘,`）
采用 **macOS 原生居中毛玻璃弹窗**（左侧分类导航 + 右侧卡片式分组表单），所有视觉与网格配置实时生效并持久化为 `~/.config/velodb/settings.toml`：
- **General & Appearance**：主题（Dark / Light / System）、等宽字体与字号、表格行高密度（Compact / Comfortable / Spacious）、毛玻璃透明效果开关。
- **Data Grid & Format**：默认 `LIMIT`、`NULL` 显示文案、时间戳时区（UTC / Local）、数值千分位格式化、外键跳转图标开关。
- **SQL Editor**：编辑器字号、行号显示、Tab 缩进宽度、关键字自动大写、默认执行策略（当前语句 / 全部语句）。
- **Safety & Query**：Safe Mode 默认开关、PROD 环境提交二次确认、单次查询超时时间、连接心跳保活。
- **Keybindings**：Vim Mode 开关、快捷键方案预设（macOS Native / TablePlus / DataGrip）与常用快捷键自定义。
- **MCP Server**：本地 MCP 启停、Agent 默认只读锁、暴露工具白名单与 `mcp_config.json` 一键复制。
- **Advanced & Config**：实时预览/导出 `settings.toml`、清理查询历史与重置默认配置。

---

## 3. Rust + GPUI 技术架构选型

| 层级 | 推荐 Crate / 方案 | 职责说明 |
| :--- | :--- | :--- |
| **UI 框架** | `gpui` (Zed) | 120fps GPU 渲染、Element/View 状态树、macOS Native Vibrancy |
| **异步与 PG 驱动** | `tokio` + `tokio-postgres` + `deadpool-postgres` | 原生异步二进制协议、连接池、Query Cancel Token |
| **SSH 隧道** | `russh` | 纯 Rust 异步 SSH 端口转发，免外部 `ssh` 进程依赖 |
| **SQL 解析与格式化** | `tree-sitter-sql` + `sqlparser-rs` | 语法高亮、多语句切分、格式化、表/列补全 |
| **本地 MCP Server** | `rmcp` (Rust MCP SDK) | 在客户端进程内暴露安全受控的 MCP 工具供外部 Agent 调用 |
| **凭据安全** | `security-framework` | macOS Keychain 加密保存数据库密码与 SSH Passphrase |
