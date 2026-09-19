//! 食物货架：她能买什么、买了回多少。
//!
//! 数据不在这里——123 项食物是 `build-assets` 从原版 `food/*.lps` 转出来的，躺在
//! Body 那边的 manifest 里。Body 启动时用 `set_food_catalog` 推给 Core，和推命中掩码
//! 一个路子。Core 只负责「按需求和钱包挑一样」，挑完把 id 塞进 `action.food`，
//! Body 照着渲染那一个精灵——而不是自己随便抓一个。

use serde::{Deserialize, Serialize};

/// 与 packages/shared/src/manifest.ts 的 `FoodItem` 一一对应
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FoodItem {
    pub id: String,
    pub name: String,
    /// eat / drink / gift —— 配哪段夹心动画
    pub graph: String,
    /// Meal / Snack / Drink / Drug / Gift / Functional
    #[serde(rename = "type")]
    pub kind: String,
    /// 前端 `/pet/` 下的 WebP，相对路径；礼物页直接用它展示缩略图
    pub src: String,
    pub strength: f32,
    /// 回多少饱腹
    pub strength_food: f32,
    /// 回多少水
    pub strength_drink: f32,
    pub feeling: f32,
    pub health: f32,
    pub price: f32,
}

/// 挑食物时优先满足哪一项
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Hunger,
    Thirst,
    /// 心情不好，买点好吃的哄自己——所以挑的东西会和单纯充饥时不一样
    Mood,
}

#[derive(Debug, Default, Clone)]
pub struct FoodShelf {
    items: Vec<FoodItem>,
}

impl FoodShelf {
    /// Startup must not depend on the pet WebView finishing its asynchronous asset load.
    pub fn bundled() -> Self {
        let items = serde_json::from_str(include_str!("../../food-catalog.json"))
            .expect("bundled food catalog must contain valid FoodItem records");
        Self { items }
    }

    pub fn gifts(&self) -> Vec<FoodItem> {
        self.items.iter().filter(|item| item.graph == "gift" && item.kind == "Gift")
            .cloned().collect()
    }

    /// 能喂的药：`Drug` 类，去掉白送的「太阳系」（价格 0、体力 −100，原版救存档用的彩蛋）
    pub fn medicines(&self) -> Vec<FoodItem> {
        self.items.iter().filter(|f| f.kind == "Drug" && f.price > 0.0).cloned().collect()
    }

    /// 缺 `deficit` 点健康时该喂哪一种：刚好够补上的里面挑最便宜的；
    /// 没有一种够的话就挑最猛的。贵的不浪费，便宜的不够用
    pub fn remedy_for(&self, deficit: f32) -> Option<&FoodItem> {
        let drugs: Vec<&FoodItem> = self.items.iter().filter(|f| f.kind == "Drug" && f.price > 0.0).collect();
        let enough = drugs.iter().copied().filter(|f| f.health >= deficit).min_by(|a, b| {
            a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal)
        });
        enough.or_else(|| {
            drugs.iter().copied().max_by(|a, b| {
                a.health.partial_cmp(&b.health).unwrap_or(std::cmp::Ordering::Equal)
            })
        })
    }

    pub fn set(&mut self, items: Vec<FoodItem>) {
        self.items = items;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, id: &str) -> Option<&FoodItem> {
        self.items.iter().find(|f| f.id == id)
    }

    /// 随机挑一件（礼物用）。`seed` 由调用方给——状态机要保持纯函数，
    /// 随机数不能长在里面
    pub fn random(&self, graph: &str, seed: u64) -> Option<&FoodItem> {
        let pool: Vec<&FoodItem> = self
            .items
            .iter()
            .filter(|f| f.graph == graph && f.kind != "Drug")
            .collect();
        if pool.is_empty() {
            return None;
        }
        Some(pool[(seed as usize) % pool.len()])
    }

    /// 按需求和钱包随机挑一样。买不起就返回 None，调用方自己决定怎么办。
    ///
    /// `graph` 是「吃」还是「喝」，和夹心动画对应。
    /// 排掉 `Drug` 和原版功能 / 彩蛋条目：它们不是自动进食的正常商品，
    /// 例如「地球」价格为 0 且会扣体力，混进来会导致她反复吃同一个异常物品。
    /// `seed` 由状态机现有状态推导，既有随机变化，又不破坏 reduce 的纯函数约束。
    pub fn pick(&self, graph: &str, need: Need, budget: f32, seed: u64) -> Option<&FoodItem> {
        let pool: Vec<&FoodItem> = self.items
            .iter()
            .filter(|f| {
                f.graph == graph
                    && f.kind != "Drug"
                    && f.price > 0.0
                    && f.strength >= 0.0
                    && f.price <= budget
                    && gain(f, need) > 0.0
            })
            .collect();
        if pool.is_empty() { None } else { Some(pool[(seed as usize) % pool.len()]) }
    }
}

fn gain(f: &FoodItem, need: Need) -> f32 {
    match need {
        Need::Hunger => f.strength_food,
        Need::Thirst => f.strength_drink,
        Need::Mood => f.feeling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 药单去掉太阳系_按缺口挑药() {
        let shelf = FoodShelf::bundled();
        let meds = shelf.medicines();
        assert_eq!(meds.len(), 9, "十种药去掉白送的太阳系");
        assert!(meds.iter().all(|m| m.price > 0.0 && m.strength >= 0.0));
        // 缺 30：够补的里面最便宜的是 大力丸（50 / 94），不是更贵的布洛芬（35 / 116）；
        // 缺 5：维生素C含片（10 / 16.5）
        assert_eq!(shelf.remedy_for(30.0).unwrap().name, "大力丸");
        assert_eq!(shelf.remedy_for(5.0).unwrap().name, "维生素C含片");
        // 缺 90 没有一种够，挑最猛的
        assert_eq!(shelf.remedy_for(90.0).unwrap().name, "速效救心丸");
    }

    #[test]
    fn bundled_gifts_available_before_webview_starts() {
        let shelf = FoodShelf::bundled();
        assert_eq!(shelf.len(), 124);
        assert_eq!(shelf.gifts().len(), 20);
        for seed in 0..20 {
            let gift = shelf.random("gift", seed).unwrap();
            assert_eq!(gift.kind, "Gift");
            assert!(!gift.name.is_empty());
            assert!(gift.src.starts_with("food/") && gift.src.ends_with(".webp"));
            assert!(shelf.get(&gift.id).is_some());
        }
    }

    #[test]
    fn 生日蛋糕已登记为可播放的食物() {
        let shelf = FoodShelf::bundled();
        let cake = shelf.get("birthday-cake").expect("生日蛋糕必须随 Core 目录加载");
        assert_eq!(cake.graph, "eat");
        assert_eq!(cake.kind, "Snack");
        assert_eq!(cake.price, 0.0);
    }

    fn item(id: &str, graph: &str, kind: &str, food: f32, drink: f32, feel: f32, price: f32) -> FoodItem {
        FoodItem {
            id: id.into(),
            name: id.into(),
            graph: graph.into(),
            kind: kind.into(),
            src: format!("food/{id}.webp"),
            strength: 0.0,
            strength_food: food,
            strength_drink: drink,
            feeling: feel,
            health: 0.0,
            price,
        }
    }

    fn shelf() -> FoodShelf {
        let mut s = FoodShelf::default();
        s.set(vec![
            item("cheap", "eat", "Snack", 20.0, 0.0, 1.0, 5.0),
            item("feast", "eat", "Meal", 60.0, 0.0, 2.0, 100.0),
            item("cake", "eat", "Snack", 10.0, 0.0, 50.0, 30.0),
            item("water", "drink", "Drink", 0.0, 40.0, 0.0, 2.0),
            item("poison", "eat", "Drug", 90.0, 0.0, 0.0, 1.0),
        ]);
        s
    }

    #[test]
    fn 随机挑礼物不会挑到空也不会挑到药() {
        let mut s = shelf();
        s.set(vec![
            item("g1", "gift", "Gift", 0.0, 0.0, 100.0, 500.0),
            item("g2", "gift", "Gift", 0.0, 0.0, 200.0, 900.0),
            item("bad", "gift", "Drug", 0.0, 0.0, 0.0, 1.0),
        ]);
        let mut seen = std::collections::HashSet::new();
        for seed in 0..20u64 {
            let f = s.random("gift", seed).unwrap();
            assert_ne!(f.kind, "Drug");
            seen.insert(f.id.clone());
        }
        assert!(seen.len() > 1, "二十次都挑到同一件，等于没随机");
    }

    #[test]
    fn 货架空了送不出礼物() {
        assert!(FoodShelf::default().random("gift", 0).is_none());
    }

    #[test]
    fn 买不起就返回空() {
        assert!(shelf().pick("eat", Need::Hunger, 1.0, 0).is_none());
    }

    #[test]
    fn 饿的时候会在买得起的食物里随机挑() {
        let s = shelf();
        let seen: std::collections::HashSet<_> = (0..12)
            .map(|seed| s.pick("eat", Need::Hunger, 999.0, seed).unwrap().id.as_str())
            .collect();
        assert!(seen.len() >= 2, "吃东西不该永远只选同一样");
    }

    #[test]
    fn 自动进食排除免费且扣体力的功能条目() {
        let shelf = FoodShelf::bundled();
        for seed in 0..500u64 {
            let item = shelf.pick("eat", Need::Hunger, 999.0, seed).unwrap();
            assert!(item.price > 0.0);
            assert!(item.strength >= 0.0);
            assert_ne!(item.name, "地球");
        }
    }

    #[test]
    fn 心情差的时候挑能哄自己的() {
        let s = shelf();
        let f = s.pick("eat", Need::Mood, 999.0, 2).unwrap();
        assert!(f.feeling > 0.0);
    }

    #[test]
    fn 渴了只在饮料里挑() {
        let s = shelf();
        let f = s.pick("drink", Need::Thirst, 999.0, 0).unwrap();
        assert_eq!(f.graph, "drink");
    }

    #[test]
    fn 永远不会去吃药() {
        let s = shelf();
        for budget in [1.0, 10.0, 999.0] {
            let picked = s.pick("eat", Need::Hunger, budget, 0);
            assert!(
                picked.is_none_or(|f| f.kind != "Drug"),
                "挑到药了（预算 {budget}）"
            );
        }
    }

    #[test]
    fn 钱包决定档次() {
        let s = shelf();
        // 只买得起便宜的
        assert_eq!(s.pick("eat", Need::Hunger, 6.0, 9).unwrap().id, "cheap");
        assert!(s.pick("eat", Need::Hunger, 999.0, 1).is_some());
    }
}
