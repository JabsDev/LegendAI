import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";

if (localStorage.getItem("legendai-theme") === "dark") {
  document.documentElement.dataset.theme = "dark";
}

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
