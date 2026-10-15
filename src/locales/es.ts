import common from "./common/es.json";
import shell from "./shell/es.json";
import connections from "./connections/es.json";
import workflows from "./workflows/es.json";

import native from "./native/es.json";

export default { ...shell, ...connections, ...workflows, ...common, ...native };
