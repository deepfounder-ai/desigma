# desigma MCP 技能卡（中文）

给 agent 看的一页纸：什么时候用 desigma、怎么开会话、工作区怎么摆、工具怎么选、坑在哪。
英文说明见 [`../agent-workflows.md`](../agent-workflows.md)。

## 适用场景

- 需要**改设计稿本身**（图层的名称、位置、颜色、文字、结构），而不是只读一份导出。
- 需要从设计稿产出**交接代码**（CSS / Tailwind / Compose / SwiftUI / SVG）。
- 需要把设计稿批量规范化：命名、组件化、实例化、页面整理。
- 不适合：纯前端实现细节、动效调优（除非用 `desigma_export_animation` 验证）。

## 两种会话

| 模式 | 判定 | 意味着什么 |
| --- | --- | --- |
| `editor` | `desigma_get_document` 返回 `"session": "editor"` | 用户在屏幕前看着；改动进**用户自己的撤销栈**；排版按编辑器规则（auto layout 回流、文字测量、连接线重排） |
| `file` | `"session": "file"` | 隔离的批处理会话；改动只在内存里，必须 `desigma_save_file` 才落盘；auto layout 不回流 |

会话模式由**第一次调用**决定，之后不会切换。editor 模式掉线要报错而不是退回 file；file 模式有未保存改动时不允许 `desigma_open_file`。

## 工作区约定

1. `--workspace` 指向仓库内目录（例如 `designs/`），所有路径都用**相对路径**；绝对路径只在工作区内部被接受。
2. 一个文件装一件事：`designs/buttons/button-kit.desigma`、`designs/screens/sign-in.desigma`。
3. 就地改打开的文件，不要每次重建文档——节点 id 只在当前文档有效，重建等于丢掉 id 映射。
4. **每次会话重新取 id**：`desigma_get_document` → `desigma_list_nodes` → `desigma_get_node`，不要复用上次的 id。
5. 导出图放同一个工作区（`designs/exports/`），因为导出路径也受工作区校验。
6. 保存策略：迭代期存成 `x.v2.desigma`；人看过之后再 `overwrite: true` 覆盖正式文件。

## 标准流程

```
desigma_list_files        → 找到文件
desigma_open_file         → {"path":"buttons/button-kit.desigma"}
desigma_get_document      → pages / session / path / has_undo
desigma_list_nodes        → {"query":"button"} 或 {"parent_id":N}
desigma_get_node          → 要改的图层全貌（含 children、fills、text、bindings）
  ……一次成组的修改……
desigma_export_image      → 看 PNG（scale 2），别只看返回值
desigma_generate_code     → 结构 / 交接代码
desigma_save_file         → 结束前必须存
```

每次修改调用 = 一步撤销；整轮可以用连续 `desigma_undo` 全部回退。

## 工具选择：`generate_code` 还是自己读节点树

- **用 `generate_code`**：图层已完成，要交接；要一份可 diff 的几何/排版参考；产物给人看。
  - `css` / `tailwind` 给后台；`compose` / `swiftui` 给客户端；`svg` 给图标。
- **自己读节点树写代码**：
  - 目标代码里**已有组件**，图层要映射成组件 + props（`Button / Primary` 是 `PrimaryButton(size: md)`，不是一堆 `Container`）；
  - 跨度超过一个图层（整屏）；
  - 布局语义重要（是否 auto layout、`layout_child` 的 grow/固定、约束关系）——生成代码会把它拍平成绝对坐标；
  - 需要改前/改后对比，或先定位图层。
- 常用分工：读树决定**建什么**，对代码库里还不存在的图层用 `generate_code`，接入已有组件和 token 的部分手写。

## 组件

- 变体族 = 一个主组件 + 实例，不要复制图层。`desigma_create_component` 把 frame 就地转主组件，保留子节点和名字。
- 改实例自己的属性 + 实例内部图层的属性，不分离：对实例内图层改 fill / opacity / text 会记成该图层的 **override**（`component.overrides` 里能看到），后续 sync 不会覆盖它。
- **改主组件会自动带到实例**（同一步撤销），不需要手动 sync。`desigma_sync_instances` 只在需要显式对账 override 时用；没有实例时返回 `{"changed": false}`，不产生撤销步。
- 只有分叉成为事实时才 `desigma_detach_instance`。

## 不要做

- editor 模式下 `discard_changes: true` 丢用户的改动。
- 一次跑完「重命名 + 换色 + 重排」——不可 review，撤销链也废了。
- 只相信返回值：批量改动前后各导一张 PNG。
- 用中文/位置当图层身份：**名字就是 API**（`Button / Primary`、`Label`、`Toolbar`），人和 agent 都靠它搜索。
