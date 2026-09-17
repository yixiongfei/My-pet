# assets-src/ —— 角色美术，不进仓库

这里放原版 [VPet-Simulator](https://github.com/LorisYounger/VPet) 的动画帧和数据，
`pnpm build:assets` 会把它们转成前端用的 WebP + `manifest.json`。**美术归原作者**，
只在本机使用，所以整个目录（除了这份说明）在 `.gitignore` 里。

克隆仓库之后要自己放进来，结构如下：

```
assets-src/
  icon.png            托盘 / 窗口图标（任意 PNG）
  pet/
    vup.lps           角色定义（触摸区域、提起锚点、work 表）
    vup/              动画帧：Default/ Sleep/ Say/ Touch_Head/ … 每个目录一组 PNG
  food/
    food.lps  drug.lps  gift.lps    食物 / 礼物表
    <图片>              食物精灵
```

来源：原版仓库的 `VPet-Simulator.Windows/mod/0000_core/pet/vup/`（`vup.lps` + 动画目录）
和 `VPet-Simulator.Windows/mod/0000_core/food/`。放好后运行：

```bash
pnpm build:assets      # 约 3–5 分钟，生成 apps/desktop/public/pet/
```
