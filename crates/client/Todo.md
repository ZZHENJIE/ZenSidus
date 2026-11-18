## 市场筛选器
通过批量订阅股票的Quote来实现

首先使用Finviz过滤出需要订阅的股票 然后进行批量订阅


把返回的Quote数据进行判断 看Ask与Bid的交易所是否为用户需要的并且要满足用户设置的股数大小才能进入结果列表

## 账号认证接入(Server 侧已定:邮箱+密码,单设备会话)

- 登录/注册窗口:email + password + 用户名(注册时),错误提示;Server 域名待定
- 会话 token 存 OS keyring(service "ZenSidus",新增条目 session-token),不落盘
- gateway_sdk:所有请求统一注入 `Authorization: Bearer <token>`;收到 401 → 清 token、提示重新登录
- 启动时 `GET /auth/me` 校验本地 token,失效则静默回到未登录态
- 设置同步(可选,后做):`user_settings` GET/PUT,revision 乐观锁;同步子集 = language/theme/watchlists/finviz 预设/translate/small_clock,window_state 不同步,API 凭据永不上云
- 前置依赖:Server 的 /auth 接口与请求/响应格式(见 Server/migrations/0001_init.sql 与 Server 仓库)
