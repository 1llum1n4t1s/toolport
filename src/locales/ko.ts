import common from "./common/ko.json";
import shell from "./shell/ko.json";
import connections from "./connections/ko.json";
import workflows from "./workflows/ko.json";

import native from "./native/ko.json";

export default { ...shell, ...connections, ...workflows, ...common, ...native };
