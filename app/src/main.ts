import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// Lets the screens be looked at in a plain browser, without the Rust side.
if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
  const { installMock } = await import("./lib/mock");
  installMock();
}

mount(App, { target: document.getElementById("app")! });
