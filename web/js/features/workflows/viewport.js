export const MIN_SCALE = 0.25;
export const MAX_SCALE = 2;
export const FIT_PADDING = 40;
export const NODE_WIDTH = 200;
export const NODE_HEIGHT = 96;

export function clampScale(scale) {
  if (!Number.isFinite(scale)) return 1;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

export function zoomAt(view, factor, point) {
  const scale = clampScale(view.scale * factor);
  const ratio = scale / view.scale;
  return {
    x: point.x - (point.x - view.x) * ratio,
    y: point.y - (point.y - view.y) * ratio,
    scale,
  };
}

export function panBy(view, dx, dy) {
  return { x: view.x + dx, y: view.y + dy, scale: view.scale };
}

export function graphBounds(nodes, options = {}) {
  const nodeWidth = options.nodeWidth || NODE_WIDTH;
  const nodeHeight = options.nodeHeight || NODE_HEIGHT;
  if (!nodes || !nodes.length) return { x: 0, y: 0, width: 0, height: 0 };
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const node of nodes) {
    const x = Number(node.x) || 0;
    const y = Number(node.y) || 0;
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x + nodeWidth);
    maxY = Math.max(maxY, y + nodeHeight);
  }
  return { x: minX, y: minY, width: Math.max(1, maxX - minX), height: Math.max(1, maxY - minY) };
}

export function fitView(nodes, viewport, options = {}) {
  const padding = options.padding === undefined ? FIT_PADDING : options.padding;
  const width = Math.max(1, viewport.width);
  const height = Math.max(1, viewport.height);
  if (!nodes || !nodes.length) return { x: padding, y: padding, scale: 1 };
  const bounds = graphBounds(nodes, options);
  const availableWidth = Math.max(1, width - padding * 2);
  const availableHeight = Math.max(1, height - padding * 2);
  const limit = options.maxScale === undefined ? 1 : options.maxScale;
  const scale = clampScale(Math.min(availableWidth / bounds.width, availableHeight / bounds.height, limit));
  return {
    x: (width - bounds.width * scale) / 2 - bounds.x * scale,
    y: (height - bounds.height * scale) / 2 - bounds.y * scale,
    scale,
  };
}

export function visibleBox(view, viewport) {
  return {
    x: (0 - view.x) / view.scale,
    y: (0 - view.y) / view.scale,
    width: Math.max(1, viewport.width) / view.scale,
    height: Math.max(1, viewport.height) / view.scale,
  };
}

export function nodeInsideViewport(node, view, viewport, options = {}) {
  const nodeWidth = options.nodeWidth || NODE_WIDTH;
  const nodeHeight = options.nodeHeight || NODE_HEIGHT;
  const box = visibleBox(view, viewport);
  const x = Number(node.x) || 0;
  const y = Number(node.y) || 0;
  return x >= box.x && y >= box.y && x + nodeWidth <= box.x + box.width && y + nodeHeight <= box.y + box.height;
}

export function autoPan(view, point, viewport, options = {}) {
  const margin = options.margin === undefined ? 48 : options.margin;
  const step = options.step === undefined ? 24 : options.step;
  let dx = 0;
  let dy = 0;
  if (point.x < margin) dx = step;
  else if (point.x > Math.max(1, viewport.width) - margin) dx = -step;
  if (point.y < margin) dy = step;
  else if (point.y > Math.max(1, viewport.height) - margin) dy = -step;
  return { view: panBy(view, dx, dy), dx, dy };
}

export function toGraphPoint(view, point) {
  return { x: (point.x - view.x) / view.scale, y: (point.y - view.y) / view.scale };
}

export function toScreenPoint(view, point) {
  return { x: point.x * view.scale + view.x, y: point.y * view.scale + view.y };
}

export function cameraTransform(view) {
  const round = (value) => Math.round(value * 1000) / 1000;
  return `translate(${round(view.x)}px, ${round(view.y)}px) scale(${round(view.scale)})`;
}

export function minimapLayout(nodes, view, viewport, options = {}) {
  const width = options.width || 160;
  const height = options.height || 110;
  const padding = options.padding === undefined ? 8 : options.padding;
  const nodeWidth = options.nodeWidth || NODE_WIDTH;
  const nodeHeight = options.nodeHeight || NODE_HEIGHT;
  const bounds = graphBounds(nodes, options);
  const scale = bounds.width && bounds.height
    ? Math.min((width - padding * 2) / bounds.width, (height - padding * 2) / bounds.height)
    : 1;
  const offsetX = (width - bounds.width * scale) / 2 - bounds.x * scale;
  const offsetY = (height - bounds.height * scale) / 2 - bounds.y * scale;
  const box = visibleBox(view, viewport);
  return {
    width,
    height,
    nodes: (nodes || []).map((node) => ({
      x: (Number(node.x) || 0) * scale + offsetX,
      y: (Number(node.y) || 0) * scale + offsetY,
      width: nodeWidth * scale,
      height: nodeHeight * scale,
    })),
    viewport: {
      x: box.x * scale + offsetX,
      y: box.y * scale + offsetY,
      width: box.width * scale,
      height: box.height * scale,
    },
  };
}

export function createViewport(options) {
  const canvas = options.canvas;
  const board = options.board;
  const listener = typeof options.onChange === "function" ? options.onChange : () => {};
  let current = options.view ? { ...options.view } : { x: 0, y: 0, scale: 1 };

  function size() {
    const rect = canvas.getBoundingClientRect ? canvas.getBoundingClientRect() : { width: 0, height: 0 };
    return {
      width: canvas.clientWidth || rect.width || 800,
      height: canvas.clientHeight || rect.height || 560,
    };
  }

  function paint() {
    if (board) {
      board.style.transformOrigin = "0 0";
      board.style.transform = cameraTransform(current);
      board.setAttribute("data-view-x", String(Math.round(current.x)));
      board.setAttribute("data-view-y", String(Math.round(current.y)));
      board.setAttribute("data-view-scale", String(current.scale));
    }
    canvas.setAttribute("data-zoom", String(Math.round(current.scale * 100)));
    listener(current, size());
  }

  function set(next) {
    current = { ...next };
    paint();
  }

  function zoom(factor, point) {
    const center = point || { x: size().width / 2, y: size().height / 2 };
    set(zoomAt(current, factor, center));
  }

  function pan(dx, dy) {
    set(panBy(current, dx, dy));
  }

  function fit(nodes, fitOptions) {
    set(fitView(nodes, size(), fitOptions));
  }

  function reset() {
    const center = { x: size().width / 2, y: size().height / 2 };
    set(zoomAt(current, 1 / current.scale, center));
  }

  function screenToGraph(point) {
    return toGraphPoint(current, point);
  }

  function graphToScreen(point) {
    return toScreenPoint(current, point);
  }

  paint();

  return {
    get view() {
      return current;
    },
    size,
    set,
    zoom,
    pan,
    fit,
    reset,
    screenToGraph,
    graphToScreen,
  };
}
