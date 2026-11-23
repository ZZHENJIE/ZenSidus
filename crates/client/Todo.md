# Todo

(2026-11-24 全部完成)

1. ✅ Chart 面板 K 线随报价实时跳动:逐笔成交实时刷新最后一根 bar 的
   close/high/low(同桶校验;volume 仍以 bar 推送为准)
2. ✅ 市场新闻去拉取,改纯推送:面板 attach 即订阅 NEWS 通道,推送驱动
   表格,无历史回填(Refresh 按钮与 backfill 已删)
3. ✅ Server 新闻按订阅推送:hub 记录 NEWS 位订阅(client 级),
   fan_out 只把 News 发给订阅了新闻的客户端
