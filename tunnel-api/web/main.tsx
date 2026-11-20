import { render } from "solid-js/web";
import { App } from "./app";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/rail.css";
import "./styles/panels.css";
import "./styles/topology.css";
import "./styles/routes.css";
import "./styles/responsive.css";
import "./styles/workspace.css";

const root = document.getElementById("root");

if (!root) throw new Error("Tunnel Owners Web requires a #root element");

render(() => <App />, root);
