# assets-src/ —— 角色美术，不进仓库

这里放原版 [VPet-Simulator](https://github.com/LorisYounger/VPet) 的动画帧和数据，
先用 `pnpm convert:pet` / `pnpm convert:food` 把原版 LPS 和目录关系归一化成 JSON，再由
`pnpm build:assets` 转成前端用的 WebP + `manifest.json`。**美术归原作者**，PNG 与原始
LPS 只在本机使用；仓库只保留这份说明和不含图片的 `pet/vup.json` 映射，方便后续优化可追踪。

克隆仓库之后要自己放进来，结构如下：

```
assets-src/
  icon.png            托盘 / 窗口图标（任意 PNG）
  pet/
    vup.lps           原版角色定义；只作一次性迁移输入
    vup.json          当前数据源：角色 profile + 全部动画 / 帧 / 夹心轨迹映射
    vup/              动画帧：Default/ Sleep/ Say/ Touch_Head/ … 每个目录一组 PNG
      **/info.lps     原版少数显式覆盖；只作一次性迁移输入
  food/
    food.json           当前数据源：礼物 / 食物 / 饮料 / 药物四层目录
    *.lps               原版表，只作一次性迁移输入
    image/*.png         食物精灵
```

来源：原版仓库的 `VPet-Simulator.Windows/mod/0000_core/pet/vup/`（`vup.lps` + 动画目录）
和 `VPet-Simulator.Windows/mod/0000_core/food/`。放好后运行：

```bash
pnpm convert:pet       # 先生成 vup.json；以后手工优化动画归类就改它
pnpm convert:food      # 生成 food.json；以后调整分类 / 数值就改它
pnpm build:assets      # 约 3–5 分钟，生成 apps/desktop/public/pet/
```

正常构建只读取版本化的 `vup.json` / `food.json`，不会再读任何 LPS。需要从原版素材重新迁移时
再次运行对应的 convert 命令（会覆盖 JSON）；结构由 `schemas/pet-source-v1.schema.json` 和
`schemas/food-source-v1.schema.json` 描述，构建时也会校验路径、枚举和文件。迁移并核对完成后，
本机的 `*.lps` 可以删除；以后若要从原版重做，重新复制原始 LPS 再执行 convert 即可。
