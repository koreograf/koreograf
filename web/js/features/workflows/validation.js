const ERROR = "error";
const WARNING = "warning";

function entry(severity, key, vars, nodes, edges) {
  return { severity, key, vars: vars || {}, nodes: nodes || [], edges: edges || [] };
}

function nodeList(graph) {
  return Array.isArray(graph && graph.nodes) ? graph.nodes : [];
}

function edgeList(graph) {
  return Array.isArray(graph && graph.edges) ? graph.edges : [];
}

function idOf(node) {
  return node && node.id != null ? String(node.id) : "";
}

export function wouldCreateCycle(graph, fromId, toId, ignoreIndex = -1) {
  if (!fromId || !toId) return false;
  if (fromId === toId) return true;
  const ids = new Set(nodeList(graph).map(idOf));
  if (!ids.has(fromId) || !ids.has(toId)) return false;
  const edges = edgeList(graph);
  const stack = [toId];
  const seen = new Set();
  while (stack.length) {
    const id = stack.pop();
    if (id === fromId) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    edges.forEach((edge, index) => {
      if (index === ignoreIndex) return;
      if (edge.from === id && ids.has(edge.to)) stack.push(edge.to);
    });
  }
  return false;
}

export function findCycleEdges(graph) {
  const ids = new Set();
  const nodes = [];
  nodeList(graph).forEach((node) => {
    const id = idOf(node);
    if (!id.trim() || ids.has(id)) return;
    ids.add(id);
    nodes.push(id);
  });
  const edges = edgeList(graph);
  const state = new Map();
  const path = [];

  function visit(id) {
    state.set(id, 1);
    for (let index = 0; index < edges.length; index += 1) {
      const edge = edges[index] || {};
      if (edge.from !== id || !ids.has(edge.to)) continue;
      const next = edge.to;
      const mark = state.get(next) || 0;
      if (mark === 1) {
        const start = path.findIndex((frame) => frame.id === next);
        const frames = start >= 0 ? path.slice(start) : [];
        return [...frames.map((frame) => frame.edge).filter((value) => value !== null), index];
      }
      if (mark === 0) {
        path.push({ id: next, edge: index });
        const found = visit(next);
        if (found) return found;
        path.pop();
      }
    }
    state.set(id, 2);
    return null;
  }

  for (const id of nodes) {
    if ((state.get(id) || 0) !== 0) continue;
    path.push({ id, edge: null });
    const found = visit(id);
    path.pop();
    if (found) return found;
  }
  return [];
}

export function analyzeGraph(graph, templates) {
  const nodes = nodeList(graph);
  const edges = edgeList(graph);
  const catalog = Array.isArray(templates) ? templates : [];
  const issues = [];
  const nodeIds = new Set();
  const seen = new Set();

  if (!nodes.length) issues.push(entry(ERROR, "the_workflow_has_no_nodes_yet", {}, [], []));

  nodes.forEach((node) => {
    const id = idOf(node);
    if (!id.trim()) {
      issues.push(entry(ERROR, "a_node_has_no_id", {}, [], []));
      return;
    }
    const duplicate = seen.has(id);
    seen.add(id);
    nodeIds.add(id);
    const templateId = node.template_id == null ? "" : String(node.template_id);
    if (!templateId.trim()) {
      issues.push(entry(ERROR, "workflow.node_template", { id }, [id], []));
    } else if (!catalog.some((template) => template && template.id === templateId)) {
      issues.push(entry(ERROR, "template.missing_named", { name: node.name || id }, [id], []));
    }
    if (!node.name || !String(node.name).trim()) {
      issues.push(entry(ERROR, "workflow.node_name", { id }, [id], []));
    }
    if (duplicate) issues.push(entry(ERROR, "workflow.node_duplicate", { id }, [id], []));
  });

  const seenEdges = new Set();
  edges.forEach((edge, index) => {
    const from = edge && edge.from;
    const to = edge && edge.to;
    if (!nodeIds.has(from) || !nodeIds.has(to)) {
      issues.push(entry(ERROR, "an_edge_points_at_an_unknown_node", {}, [], [index]));
      return;
    }
    if (from === to) {
      issues.push(entry(ERROR, "an_edge_cannot_point_at_the_same_node", {}, [], [index]));
      return;
    }
    const key = `${from}->${to}:${String(edge.when)}`;
    if (seenEdges.has(key)) {
      issues.push(entry(ERROR, "the_same_edge_is_present_twice", {}, [], [index]));
    } else {
      seenEdges.add(key);
    }
  });

  const cycle = findCycleEdges(graph);
  if (cycle.length) {
    issues.push(entry(ERROR, "the_workflow_contains_a_cycle", {}, [], cycle));
  }

  if (nodes.length) {
    const incoming = new Map();
    const outgoing = new Map();
    nodeIds.forEach((id) => {
      incoming.set(id, 0);
      outgoing.set(id, 0);
    });
    edges.forEach((edge) => {
      if (!nodeIds.has(edge.from) || !nodeIds.has(edge.to)) return;
      outgoing.set(edge.from, (outgoing.get(edge.from) || 0) + 1);
      incoming.set(edge.to, (incoming.get(edge.to) || 0) + 1);
    });
    nodes.forEach((node) => {
      const id = idOf(node);
      if (!nodeIds.has(id)) return;
      if ((incoming.get(id) || 0) === 0 && (outgoing.get(id) || 0) === 0) {
        issues.push(entry(WARNING, "node_is_unconnected", { name: node.name || id }, [id], []));
      }
    });
    const roots = [...nodeIds].filter((id) => (incoming.get(id) || 0) === 0);
    const reachable = new Set();
    const stack = [...roots];
    while (stack.length) {
      const id = stack.pop();
      if (reachable.has(id)) continue;
      reachable.add(id);
      edges.forEach((edge) => {
        if (edge.from === id && nodeIds.has(edge.to) && !reachable.has(edge.to)) stack.push(edge.to);
      });
    }
    nodes.forEach((node) => {
      const id = idOf(node);
      if (!nodeIds.has(id) || reachable.has(id)) return;
      issues.push(entry(WARNING, "node_is_unreachable", { name: node.name || id }, [id], []));
    });
  }

  const nodeIssues = {};
  const edgeIssues = {};
  issues.forEach((issue) => {
    issue.nodes.forEach((id) => {
      if (!nodeIssues[id]) nodeIssues[id] = [];
      nodeIssues[id].push(issue);
    });
    issue.edges.forEach((index) => {
      if (!edgeIssues[index]) edgeIssues[index] = [];
      edgeIssues[index].push(issue);
    });
  });

  return {
    issues,
    errors: issues.filter((issue) => issue.severity === ERROR),
    warnings: issues.filter((issue) => issue.severity === WARNING),
    nodeIssues,
    edgeIssues,
  };
}

export function issuesForServerError(graph, key, vars) {
  const values = vars || {};
  const nodes = nodeList(graph);
  const edges = edgeList(graph);
  const ids = new Set(nodes.map(idOf));
  const make = (nodesList, edgesList) => ({
    severity: ERROR,
    key,
    vars: values,
    nodes: nodesList || [],
    edges: edgesList || [],
    server: true,
  });

  if (key === "the_workflow_contains_a_cycle") return [make([], findCycleEdges(graph))];
  if (key === "an_edge_points_at_an_unknown_node") {
    const list = [];
    edges.forEach((edge, index) => {
      if (!ids.has(edge.from) || !ids.has(edge.to)) list.push(index);
    });
    return [make([], list)];
  }
  if (key === "an_edge_cannot_point_at_the_same_node") {
    const list = [];
    edges.forEach((edge, index) => {
      if (edge.from === edge.to) list.push(index);
    });
    return [make([], list)];
  }
  if (key === "the_same_edge_is_present_twice") {
    const seen = new Set();
    const list = [];
    edges.forEach((edge, index) => {
      const edgeKey = `${edge.from}->${edge.to}:${String(edge.when)}`;
      if (seen.has(edgeKey)) list.push(index);
      else seen.add(edgeKey);
    });
    return [make([], list)];
  }
  if (key === "workflow.node_template" || key === "workflow.node_name" || key === "workflow.node_duplicate") {
    const id = values.id == null ? "" : String(values.id);
    return [make(id ? [id] : [], [])];
  }
  if (key === "template.missing_named") {
    const name = values.name == null ? "" : String(values.name);
    const node = nodes.find((item) => String(item.name || "") === name);
    return [make(node ? [idOf(node)] : [], [])];
  }
  if (key === "a_node_has_no_id" || key === "the_workflow_has_no_nodes_yet") return [make([], [])];
  return [];
}
