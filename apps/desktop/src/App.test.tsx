import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { App } from "./App";

test("the shell exposes the standalone CLI version command", () => {
  const html = renderToStaticMarkup(<App />);
  expect(html).toContain("<h1>Envlume</h1>");
  expect(html).toContain("<code>envlume --version</code>");
});
