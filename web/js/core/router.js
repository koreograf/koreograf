function split(value) {
  const path = String(value || "").replace(/^#/, "").split("?")[0];
  return path.split("/").filter(Boolean);
}

function fillTarget(target, params) {
  return target.replace(/:([A-Za-z0-9_]+)/g, (_, name) => encodeURIComponent(params[name] ?? ""));
}

export function createRouter() {
  const routes = [];

  function registerRoute(pattern, handler) {
    routes.push({ pattern, parts: split(pattern), handler });
  }

  function registerRedirect(pattern, target) {
    registerRoute(pattern, (params) => ({
      redirect: typeof target === "function" ? target(params) : fillTarget(target, params),
    }));
  }

  function matchRoute(path) {
    const parts = split(path);
    for (const route of routes) {
      if (route.parts.length !== parts.length) continue;
      const params = {};
      let matched = true;
      for (let index = 0; index < route.parts.length; index += 1) {
        const part = route.parts[index];
        if (part.startsWith(":")) {
          params[part.slice(1)] = parts[index];
        } else if (part !== parts[index]) {
          matched = false;
          break;
        }
      }
      if (matched) return { pattern: route.pattern, handler: route.handler, params };
    }
    return null;
  }

  return { registerRoute, registerRedirect, matchRoute };
}

const defaultRouter = createRouter();

export const registerRoute = defaultRouter.registerRoute;
export const registerRedirect = defaultRouter.registerRedirect;
export const matchRoute = defaultRouter.matchRoute;
