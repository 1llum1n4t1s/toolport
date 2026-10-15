# Product language

Use plain, direct UI language. Keep Toolport, Pro, MCP, OAuth, API, JSON,
client and provider product names, protocol methods, code, URLs, paths and
placeholders unchanged. Translate server/client only as the terms below, never
as a restaurant server or a customer. A tool is a callable MCP operation.
A catalog is the available tools and their schemas, not a shopping catalog.
A call invokes a tool; it is not a phone call. A check validates something;
it is not a bank check. A team seat is a licensed member, not a chair.

Savings are model input that Toolport avoided sending. They are not stored
records, stored credentials, or billing savings. A negative net value means
more tokens were sent than the baseline. Preserve signs and the baseline minus
catalog minus search equation. Auth tokens are credentials and use their own
wording. Never use 代币, 代幣, 權杖, 权杖 or 令牌 for LLM tokens.

| Concept            | English             | 日本語                            | 한국어                    | 简体中文               | 繁體中文               |
| ------------------ | ------------------- | --------------------------------- | ------------------------- | ---------------------- | ---------------------- |
| LLM token(s)       | token(s)            | Token                             | Token                     | Token                  | Token                  |
| tokens saved       | tokens saved        | 削減したToken数                   | 절감한 Token              | 节省的 Token           | 節省的 Token           |
| net tokens saved   | net tokens saved    | 正味のToken削減数                 | 순 절감 Token             | 净节省的 Token         | 淨節省的 Token         |
| auth token         | access/bearer token | アクセストークン / Bearerトークン | 액세스 토큰 / Bearer 토큰 | 访问令牌 / Bearer 令牌 | 存取權杖 / Bearer 權杖 |
| server             | server              | サーバー                          | 서버                      | 服务器                 | 伺服器                 |
| client             | client              | クライアント                      | 클라이언트                | 客户端                 | 用戶端                 |
| gateway            | gateway             | ゲートウェイ                      | 게이트웨이                | 网关                   | 閘道                   |
| tool               | tool                | ツール                            | 도구                      | 工具                   | 工具                   |
| catalog            | catalog             | カタログ                          | 카탈로그                  | 目录                   | 目錄                   |
| quarantine         | quarantine          | 隔離                              | 격리                      | 隔离                   | 隔離                   |
| approve / approval | approve / approval  | 承認する / 承認                   | 승인 / 승인               | 批准 / 批准            | 核准 / 核准            |
| profile            | profile             | プロファイル                      | 프로필                    | 配置方案               | 設定檔                 |
| sync / Pro sync    | sync / Pro sync     | 同期 / Pro同期                    | 동기화 / Pro 동기화       | 同步 / Pro 同步        | 同步 / Pro 同步        |
| team               | team                | チーム                            | 팀                        | 团队                   | 團隊                   |
| secret             | secret              | シークレット                      | 시크릿                    | 机密值                 | 機密值                 |
| password manager   | password manager    | パスワードマネージャー            | 비밀번호 관리자           | 密码管理器             | 密碼管理器             |
| Code Mode          | Code Mode           | Code Mode                         | Code Mode                 | Code Mode              | Code Mode              |
| discovery          | discovery           | ツール探索                        | 도구 탐색                 | 工具发现               | 工具探索               |
| search             | search              | 検索                              | 검색                      | 搜索                   | 搜尋                   |

| Concept            | Deutsch                                 | Français                              | Español                             | Português (Brasil)                |
| ------------------ | --------------------------------------- | ------------------------------------- | ----------------------------------- | --------------------------------- |
| LLM token(s)       | Token                                   | token(s)                              | token(s)                            | token(s)                          |
| tokens saved       | eingesparte Token                       | tokens économisés                     | tokens ahorrados                    | tokens economizados               |
| net tokens saved   | netto eingesparte Token                 | économie nette de tokens              | ahorro neto de tokens               | economia líquida de tokens        |
| auth token         | Zugriffstoken / Bearer-Token            | jeton d’accès / jeton Bearer          | token de acceso / token Bearer      | token de acesso / token Bearer    |
| server             | Server                                  | serveur                               | servidor                            | servidor                          |
| client             | Client                                  | client                                | cliente                             | cliente                           |
| gateway            | Gateway                                 | passerelle                            | gateway                             | gateway                           |
| tool               | Tool                                    | outil                                 | herramienta                         | ferramenta                        |
| catalog            | Katalog                                 | catalogue                             | catálogo                            | catálogo                          |
| quarantine         | Quarantäne                              | quarantaine                           | cuarentena                          | quarentena                        |
| approve / approval | genehmigen / Genehmigung                | approuver / approbation               | aprobar / aprobación                | aprovar / aprovação               |
| profile            | Profil                                  | profil                                | perfil                              | perfil                            |
| sync / Pro sync    | Synchronisierung / Pro-Synchronisierung | synchronisation / synchronisation Pro | sincronización / sincronización Pro | sincronização / sincronização Pro |
| team               | Team                                    | équipe                                | equipo                              | equipe                            |
| secret             | geheimer Wert                           | secret                                | secreto                             | segredo                           |
| password manager   | Passwortmanager                         | gestionnaire de mots de passe         | gestor de contraseñas               | gerenciador de senhas             |
| Code Mode          | Code Mode                               | Code Mode                             | Code Mode                           | Code Mode                         |
| discovery          | Tool-Erkennung                          | découverte des outils                 | descubrimiento de herramientas      | descoberta de ferramentas         |
| search             | Suche                                   | recherche                             | búsqueda                            | busca                             |

Inflect these terms naturally. Keep the distinction between discovery (the
client obtaining tools) and search (a query for matching tools). Existing auth
protocol terminology may use localized token names; the LLM-token prohibition
does not apply to those credentials. Product labels Code Mode and Pro remain
unchanged. User-provided names, tool names and historical server output remain
user data, not translation source strings.
