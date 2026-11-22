//! K 线图组件(纯自定义)。
//!
//! gpui-component 自带的 `CandlestickChart` 是分类(band)坐标轴的静态图:
//! 无平移缩放、无实时推送更新、无成交量副图、无十字光标,不满足看盘需求,
//! 因此这里的绘制与交互全部自实现,不依赖任何图表组件。
//!
//! 分层(行为与呈现同在本 crate,应用侧只拿 [`Kline`] + [`KlineState`]):
//! - [`state`] 状态实体:历史 bars、视口、实时推送聚合([`Timeframe`] 定义);
//! - [`element`] 自定义 Element:三阶段绘制 + 指针交互;
//! - [`scale`] 纯几何/格式化函数(可单测):价格刻度、时间桶、美东标签。
//!
//! 数据入口两条线,由面板接好:
//! - 历史:`RpcRequestPayload::Bars` → [`load_history`](KlineState::set_history);
//! - 实时:hub 的 `DailyBar`(日线)/ `MinuteBar/UpdatedBar`(分钟周期,
//!   客户端聚合成所选周期)。

use anyhow::anyhow;
use gpui_kit::{
    App, SharedString, WeakEntity,
    component::menu::{PopupMenu, PopupMenuItem},
};

use protocol::{RpcRequestPayload, RpcResponsePayload};

use crate::tr;
use crate::utils::message;
use crate::utils::state::AppState;

mod element;
mod scale;
mod state;

pub use element::Kline;
pub use state::{KlineState, Timeframe};

/// 拉取一个 K 线状态的 Bars 历史(标的与周期读自状态本身)。离线静默跳过
/// (启动自动加载不进消息中心,连接建立后由调用方补载);响应代数不符即
/// 丢弃;失败进消息中心。
pub fn load_history(entity: WeakEntity<KlineState>, cx: &mut App) {
    let Ok(server) = AppState::server(cx) else {
        return;
    };
    let Some((symbol, timeframe, generation)) = entity
        .update(cx, |state, _| {
            let symbol = state.symbol().map(str::to_owned)?;
            let timeframe = state.timeframe();
            Some((symbol, timeframe, state.begin_load()))
        })
        .ok()
        .flatten()
    else {
        return;
    };
    cx.spawn(async move |cx| {
        let result = server
            .rpc_checked(RpcRequestPayload::Bars {
                symbols: vec![symbol.clone()],
                timeframe: timeframe.as_str().to_string(),
                start: None,
                end: None,
                limit: timeframe.history_limit(),
            })
            .await;
        let outcome = match result {
            Ok(RpcResponsePayload::Bars(map)) => Ok(map.get(&symbol).cloned().unwrap_or_default()),
            Ok(other) => Err(anyhow!("unexpected rpc response: {other:?}")),
            Err(err) => Err(err),
        };
        let _ = entity.update(cx, |state, cx| match outcome {
            Ok(bars) => {
                if state.set_history(generation, bars) {
                    cx.notify();
                }
            }
            Err(err) => {
                if state.fail_load(generation) {
                    cx.notify();
                }
                message::error(tr!("app.chart.load_failed"), err, cx);
            }
        });
    })
    .detach();
}

/// 菜单项文案:数值 + 单位词(本地化),如 "5 Minutes"/"5 分钟"、"1 Day"/"1 天"。
pub fn timeframe_item_label(timeframe: Timeframe) -> SharedString {
    match timeframe {
        Timeframe::OneDay => tr!("app.chart.tf_day"),
        Timeframe::OneWeek => tr!("app.chart.tf_week"),
        Timeframe::OneMonth => tr!("app.chart.tf_month"),
        tf if tf.bucket_minutes() < 60 => tr!("app.chart.tf_minute", n = tf.bucket_minutes()),
        tf => tr!("app.chart.tf_hour", n = tf.bucket_minutes() / 60),
    }
}

/// 周期选择菜单:扁平列表,每项 = 数值 + 单位词,单位切换处插分隔线;
/// 选中项打勾。`on_pick` 在选中时回调。
pub fn timeframe_menu(
    current: Timeframe,
    on_pick: impl Fn(Timeframe, &mut App) + Clone + 'static,
    menu: PopupMenu,
) -> PopupMenu {
    // 单位词的 locale 键:单位变了就插一条分隔线。
    let unit_key = |tf: Timeframe| -> &'static str {
        match tf {
            Timeframe::OneDay => "app.chart.tf_day",
            Timeframe::OneWeek => "app.chart.tf_week",
            Timeframe::OneMonth => "app.chart.tf_month",
            tf if tf.bucket_minutes() < 60 => "app.chart.tf_minute",
            _ => "app.chart.tf_hour",
        }
    };

    let mut menu = menu;
    let mut last_unit: Option<&'static str> = None;
    for timeframe in Timeframe::ALL {
        let key = unit_key(timeframe);
        if last_unit.is_some_and(|unit| unit != key) {
            menu = menu.separator();
        }
        last_unit = Some(key);

        let on_pick = on_pick.clone();
        menu = menu.item(
            PopupMenuItem::new(timeframe_item_label(timeframe))
                .checked(timeframe == current)
                .on_click(move |_, _, cx| on_pick(timeframe, cx)),
        );
    }
    menu
}
