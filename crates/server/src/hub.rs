//! 订阅聚合:per-client 意愿表 → 上游并集(引用计数语义)。与客户端 hub
//! 同构——多面板同看 NVDA 只订一次;客户端断开撤销其全部意愿。
//!
//! 客户端发来的是增量(`Subscribe`/`Unsubscribe` 携带 symbols + channels),
//! 服务端按 (client, symbol) 维度累积通道位;每次变更重算全量并集并与
//! "已发上游"镜像做差,差量才是要发给 anysdk 的 subscribe/unsubscribe。

use std::collections::{BTreeSet, HashMap};

use anysdk::alpaca_stream::StockSubscription;
use protocol::Channels;

/// 一个客户端的意愿变更后,上游需要的差量。
#[derive(Debug, Default)]
pub struct HubDiff {
    pub subscribe: StockSubscription,
    pub unsubscribe: StockSubscription,
}

impl HubDiff {
    /// 测试判定差量为空(生产路径按 sub/unsub 各自非空下发)。
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        subscription_is_empty(&self.subscribe) && subscription_is_empty(&self.unsubscribe)
    }
}

/// `StockSubscription::is_empty` 在 anysdk 里是私有的,这里自写同语义判定。
pub(crate) fn subscription_is_empty(subscription: &StockSubscription) -> bool {
    subscription.trades.is_empty()
        && subscription.quotes.is_empty()
        && subscription.bars.is_empty()
        && subscription.updated_bars.is_empty()
        && subscription.daily_bars.is_empty()
        && subscription.statuses.is_empty()
        && subscription.lulds.is_empty()
        && subscription.imbalances.is_empty()
}

#[derive(Debug, Default)]
pub struct SubscriptionHub {
    /// client_id → (symbol → channels)。符号键统一大写(行情帧符号全
    /// 大写;客户端订阅串大小写容错,入库归一让路由查表 O(1))。
    wishes: HashMap<u64, HashMap<String, Channels>>,
    /// 已发给上游的并集镜像(diff 的基准)。
    upstream: HashMap<String, Channels>,
}

/// 上游并集的按通道符号集合(BARS 同时展开到 bars 与 updated_bars 两个
/// 上游字段)。
#[derive(Debug, Default, PartialEq)]
struct FieldSets {
    trades: BTreeSet<String>,
    quotes: BTreeSet<String>,
    bars: BTreeSet<String>,
    updated_bars: BTreeSet<String>,
    daily_bars: BTreeSet<String>,
}

impl FieldSets {
    fn of(map: &HashMap<String, Channels>) -> Self {
        let mut sets = Self::default();
        for (symbol, channels) in map {
            if channels.contains(Channels::TRADES) {
                sets.trades.insert(symbol.clone());
            }
            if channels.contains(Channels::QUOTES) {
                sets.quotes.insert(symbol.clone());
            }
            if channels.contains(Channels::BARS) {
                sets.bars.insert(symbol.clone());
                sets.updated_bars.insert(symbol.clone());
            }
            if channels.contains(Channels::DAILY) {
                sets.daily_bars.insert(symbol.clone());
            }
        }
        sets
    }
}

fn subset(a: &BTreeSet<String>, b: &BTreeSet<String>) -> Vec<String> {
    a.difference(b).cloned().collect()
}

impl SubscriptionHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 客户端增量订阅:symbols 的通道位累加进该 client 的意愿(符号键
    /// 归一大写)。
    pub fn apply_subscribe(
        &mut self,
        client_id: u64,
        symbols: &[String],
        channels: Channels,
    ) -> HubDiff {
        let wish = self.wishes.entry(client_id).or_default();
        for symbol in symbols {
            let bits = wish
                .entry(symbol.to_ascii_uppercase())
                .or_insert(Channels::empty());
            *bits |= channels;
        }
        self.diff()
    }

    /// 客户端增量退订:清除通道位;位清空即移除该 (client, symbol) 意愿。
    pub fn apply_unsubscribe(
        &mut self,
        client_id: u64,
        symbols: &[String],
        channels: Channels,
    ) -> HubDiff {
        if let Some(wish) = self.wishes.get_mut(&client_id) {
            for symbol in symbols {
                let key = symbol.to_ascii_uppercase();
                if let Some(bits) = wish.get_mut(key.as_str()) {
                    *bits &= !channels;
                    if bits.is_empty() {
                        wish.remove(key.as_str());
                    }
                }
            }
            if wish.is_empty() {
                self.wishes.remove(&client_id);
            }
        }
        self.diff()
    }

    /// 路由判定(fan_out 纯分发用):该客户端在该符号上是否有任一指定
    /// 通道位。符号大小写不敏感(入库已归一大写,查表入参同样归一)。
    pub fn subscribed(&self, client_id: u64, symbol: &str, channels: Channels) -> bool {
        self.wishes
            .get(&client_id)
            .and_then(|wish| wish.get(symbol.to_ascii_uppercase().as_str()))
            .is_some_and(|bits| bits.intersects(channels))
    }

    /// 客户端断开:撤销其全部意愿。
    pub fn remove_client(&mut self, client_id: u64) -> HubDiff {
        self.wishes.remove(&client_id);
        self.diff()
    }

    fn diff(&mut self) -> HubDiff {
        let union = self.union();
        let old = FieldSets::of(&self.upstream);
        let new = FieldSets::of(&union);

        let mut diff = HubDiff::default();
        diff.subscribe.trades = subset(&new.trades, &old.trades);
        diff.unsubscribe.trades = subset(&old.trades, &new.trades);
        diff.subscribe.quotes = subset(&new.quotes, &old.quotes);
        diff.unsubscribe.quotes = subset(&old.quotes, &new.quotes);
        diff.subscribe.bars = subset(&new.bars, &old.bars);
        diff.unsubscribe.bars = subset(&old.bars, &new.bars);
        diff.subscribe.updated_bars = subset(&new.updated_bars, &old.updated_bars);
        diff.unsubscribe.updated_bars = subset(&old.updated_bars, &new.updated_bars);
        diff.subscribe.daily_bars = subset(&new.daily_bars, &old.daily_bars);
        diff.unsubscribe.daily_bars = subset(&old.daily_bars, &new.daily_bars);

        self.upstream = union;
        diff
    }

    /// 全部客户端意愿的并集。
    fn union(&self) -> HashMap<String, Channels> {
        let mut union: HashMap<String, Channels> = HashMap::new();
        for wish in self.wishes.values() {
            for (symbol, channels) in wish {
                let bits = union.entry(symbol.clone()).or_insert(Channels::empty());
                *bits |= *channels;
            }
        }
        union
    }

    /// 当前上游并集(测试用;生产日志暂未消费)。
    #[cfg(test)]
    pub fn upstream(&self) -> &HashMap<String, Channels> {
        &self.upstream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn subscribe_then_unsubscribe_cancels_out() {
        let mut hub = SubscriptionHub::new();
        let diff = hub.apply_subscribe(1, &symbols(&["AAPL"]), Channels::TRADES | Channels::DAILY);
        assert_eq!(diff.subscribe.trades, ["AAPL"]);
        assert_eq!(diff.subscribe.daily_bars, ["AAPL"]);
        assert!(subscription_is_empty(&diff.unsubscribe));

        let diff =
            hub.apply_unsubscribe(1, &symbols(&["AAPL"]), Channels::TRADES | Channels::DAILY);
        assert_eq!(diff.unsubscribe.trades, ["AAPL"]);
        assert_eq!(diff.unsubscribe.daily_bars, ["AAPL"]);
        assert!(subscription_is_empty(&diff.subscribe));
        assert!(hub.upstream().is_empty());
    }

    /// 两个客户端同看 NVDA:并集只订一次;其一断开,上游不动。
    #[test]
    fn shared_symbols_refcount_across_clients() {
        let mut hub = SubscriptionHub::new();
        hub.apply_subscribe(1, &symbols(&["NVDA"]), Channels::TRADES | Channels::DAILY);
        let diff = hub.apply_subscribe(2, &symbols(&["NVDA"]), Channels::TRADES | Channels::DAILY);
        assert!(diff.is_empty(), "second subscriber adds nothing upstream");

        let diff = hub.remove_client(1);
        assert!(
            diff.is_empty(),
            "client 1 leaving changes nothing: 2 still wants NVDA"
        );

        let diff = hub.remove_client(2);
        assert_eq!(diff.unsubscribe.trades, ["NVDA"]);
        assert_eq!(diff.unsubscribe.daily_bars, ["NVDA"]);
        assert!(hub.upstream().is_empty());
    }

    /// 通道按位累积:A 订 trades,B 订 quotes → 并集两者;B 退订只撤 quotes。
    #[test]
    fn channels_union_per_symbol() {
        let mut hub = SubscriptionHub::new();
        hub.apply_subscribe(1, &symbols(&["TSLA"]), Channels::TRADES);
        let diff = hub.apply_subscribe(2, &symbols(&["TSLA"]), Channels::QUOTES | Channels::BARS);
        // B 新增的只有 quotes/bars/updated_bars;trades 不重发。
        assert!(diff.subscribe.trades.is_empty());
        assert_eq!(diff.subscribe.quotes, ["TSLA"]);
        assert_eq!(diff.subscribe.bars, ["TSLA"]);
        assert_eq!(diff.subscribe.updated_bars, ["TSLA"]);

        let diff = hub.apply_unsubscribe(
            2,
            &symbols(&["TSLA"]),
            Channels::QUOTES | Channels::BARS | Channels::DAILY,
        );
        assert_eq!(diff.unsubscribe.quotes, ["TSLA"]);
        assert_eq!(diff.unsubscribe.bars, ["TSLA"]);
        assert_eq!(diff.unsubscribe.updated_bars, ["TSLA"]);
        assert!(
            diff.unsubscribe.trades.is_empty(),
            "client 1 still wants trades"
        );
        assert!(hub.upstream()["TSLA"].contains(Channels::TRADES));
    }

    /// 订阅同 symbol 同通道两次,一次退订不撤上游(引用计数)。
    #[test]
    fn duplicate_subscribe_counts_once_upstream_but_tracks_both() {
        let mut hub = SubscriptionHub::new();
        hub.apply_subscribe(1, &symbols(&["AAPL"]), Channels::QUOTES);
        let diff = hub.apply_subscribe(1, &symbols(&["AAPL"]), Channels::QUOTES);
        assert!(diff.is_empty());

        // 第一次退订:client 1 的位被清空(同一客户端同 symbol 同通道只有一份)。
        let diff = hub.apply_unsubscribe(1, &symbols(&["AAPL"]), Channels::QUOTES);
        assert_eq!(diff.unsubscribe.quotes, ["AAPL"]);
    }

    /// 断开未知客户端 = 无操作。
    #[test]
    fn removing_unknown_client_is_noop() {
        let mut hub = SubscriptionHub::new();
        assert!(hub.remove_client(99).is_empty());
    }

    /// 路由判定:组合位各通道独立命中;符号大小写归一;缺位/缺客户端/
    /// 未订阅符号返回 false。
    #[test]
    fn subscribed_routes_by_symbol_and_channel() {
        let mut hub = SubscriptionHub::new();
        hub.apply_subscribe(1, &symbols(&["AAPL"]), Channels::TRADES | Channels::DAILY);
        hub.apply_subscribe(1, &symbols(&["nvda"]), Channels::QUOTES);

        assert!(hub.subscribed(1, "AAPL", Channels::TRADES));
        assert!(hub.subscribed(1, "AAPL", Channels::DAILY));
        assert!(!hub.subscribed(1, "AAPL", Channels::QUOTES));
        assert!(
            hub.subscribed(1, "NVDA", Channels::QUOTES),
            "小写入库归一为大写,大写查表命中"
        );
        assert!(!hub.subscribed(1, "NVDA", Channels::TRADES));
        assert!(!hub.subscribed(2, "AAPL", Channels::TRADES), "未知客户端");
        assert!(!hub.subscribed(1, "TSLA", Channels::TRADES), "未订阅符号");

        // 退订后路由即断(大写归一的退订路径同样命中)。
        hub.apply_unsubscribe(1, &symbols(&["AAPL"]), Channels::DAILY);
        assert!(!hub.subscribed(1, "AAPL", Channels::DAILY));
        assert!(hub.subscribed(1, "AAPL", Channels::TRADES), "其余位保留");
    }
}
