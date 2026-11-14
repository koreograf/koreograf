export const GRID_SIZE = 22;
export const NODE_WIDTH = 200;
export const NODE_HEIGHT = 132;
export const COLUMN_STEP = NODE_WIDTH + 64;
export const ROW_STEP = NODE_HEIGHT + 44;

export function snapToGrid(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return 0;
  return Math.round(number / GRID_SIZE) * GRID_SIZE;
}

function graphEdges(graph) {
  return Array.isArray(graph && graph.edges) ? graph.edges : [];
}

function graphNodes(graph) {
  return Array.isArray(graph && graph.nodes) ? graph.nodes : [];
}

function layerOrder(ids, incoming, outgoing) {
  const indegree = new Map(ids.map((id) => [id, incoming.get(id).length]));
  const layerOf = new Map(ids.map((id) => [id, 0]));
  const queue = ids.filter((id) => indegree.get(id) === 0);
  const placed = new Set();
  while (queue.length) {
    const id = queue.shift();
    placed.add(id);
    outgoing.get(id).forEach((next) => {
      layerOf.set(next, Math.max(layerOf.get(next), layerOf.get(id) + 1));
      indegree.set(next, indegree.get(next) - 1);
      if (indegree.get(next) === 0) queue.push(next);
    });
  }
  ids.filter((id) => !placed.has(id)).forEach((id, position, pending) => {
    const earlier = new Set(pending.slice(0, position));
    const known = incoming.get(id).filter((pred) => placed.has(pred) || earlier.has(pred));
    const base = known.length ? Math.max(...known.map((pred) => layerOf.get(pred))) + 1 : 0;
    layerOf.set(id, base);
  });
  const count = Math.max(0, ...ids.map((id) => layerOf.get(id))) + 1;
  const layers = Array.from({ length: count }, () => []);
  ids.forEach((id) => layers[layerOf.get(id)].push(id));
  return layers;
}

function positionsIn(layers) {
  const positions = new Map();
  layers.forEach((layer) => layer.forEach((id, position) => positions.set(id, position)));
  return positions;
}

function barycenter(id, neighbors, positions) {
  const values = neighbors.filter((neighbor) => positions.has(neighbor)).map((neighbor) => positions.get(neighbor));
  if (!values.length) return positions.get(id);
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function orderLayers(layers, incoming, outgoing) {
  let positions = positionsIn(layers);
  const sortLayer = (layer, neighbors) => {
    const scores = new Map(layer.map((id) => [id, barycenter(id, neighbors.get(id), positions)]));
    layer.sort((left, right) => scores.get(left) - scores.get(right) || positions.get(left) - positions.get(right));
    positions = positionsIn(layers);
  };
  for (let pass = 0; pass < 4; pass += 1) {
    for (let index = 1; index < layers.length; index += 1) sortLayer(layers[index], incoming);
    for (let index = layers.length - 2; index >= 0; index -= 1) sortLayer(layers[index], outgoing);
  }
}

export function layoutGraph(graph, options = {}) {
  const nodes = graphNodes(graph);
  if (!nodes.length) return [];
  const columnStep = options.columnStep || COLUMN_STEP;
  const rowStep = options.rowStep || ROW_STEP;
  const index = new Map();
  const ids = nodes.map((node, position) => {
    const id = String(node.id);
    index.set(id, position);
    return id;
  });
  const incoming = new Map(ids.map((id) => [id, []]));
  const outgoing = new Map(ids.map((id) => [id, []]));
  graphEdges(graph).forEach((edge) => {
    const from = String(edge.from);
    const to = String(edge.to);
    if (!incoming.has(to) || !outgoing.has(from) || from === to) return;
    if (!incoming.get(to).includes(from)) incoming.get(to).push(from);
    if (!outgoing.get(from).includes(to)) outgoing.get(from).push(to);
  });
  const layers = layerOrder(ids, incoming, outgoing);
  orderLayers(layers, incoming, outgoing);
  const tallest = Math.max(...layers.map((layer) => layer.length));
  const positions = new Map();
  layers.forEach((layer, layerIndex) => {
    const offset = ((tallest - layer.length) * rowStep) / 2;
    layer.forEach((id, row) => {
      positions.set(id, {
        id: nodes[index.get(id)].id,
        x: snapToGrid(layerIndex * columnStep),
        y: snapToGrid(offset + row * rowStep),
      });
    });
  });
  return ids.map((id) => positions.get(id));
}
