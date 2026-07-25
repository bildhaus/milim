import { Suspense } from "react";
import { renderToString } from "react-dom/server";
import { LandingPage } from "./App";

export function renderLandingPage() {
  return renderToString(<LandingPage />);
}

export async function renderDocsPage(pathname: string) {
  const { DocsPage } = await import("./DocsPage");
  return renderToString(
    <Suspense fallback={null}>
      <DocsPage pathname={pathname} />
    </Suspense>,
  );
}
