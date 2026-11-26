// A deterministic stand-in for mm-serve. It serves the retained routes and,
// on POST /api/turns, streams one complete action turn over SSE.
import { createServer } from "node:http";

const port = Number(process.env.MOCK_PORT ?? 8897);
const sessionId = "s-demo";
const clients = new Set();
let seq = 1;

const stored = [
  event(seq++, "session.started", { userId: "operator", title: "Demo session" }, undefined),
];

function event(number, type, data, turnId = "t-demo") {
  return {
    v: 2,
    seq: number,
    id: `evt-${String(number).padStart(4, "0")}`,
    sessionId,
    ...(turnId ? { turnId } : {}),
    at: Date.now(),
    type,
    source: "mock",
    data,
  };
}

function tool(name, description, parameters) {
  return { name, description, parameters };
}

const tools = [
  tool("conversation.set_topic", "Set the conversation topic.", {
    type: "object",
    properties: { topic: { type: "string" } },
    required: ["topic"],
  }),
  tool("conversation.set_depth", "Set the conversation depth.", {
    type: "object",
    properties: { depth: { type: "integer" } },
    required: ["depth"],
  }),
  tool("metamind.verify", "Register a check the turn must satisfy.", {
    type: "object",
    properties: { checks: { type: "array", items: { type: "object" } } },
    required: ["checks"],
  }),
  tool("metamind.approve", "Approve the staged state.", {
    type: "object",
    properties: { reason: { type: "string" } },
  }),
];

function messages(pass) {
  return [
    { role: "system", content: `You are Metamind. FULL SYSTEM PROMPT for ${pass}.` },
    { role: "system", content: "[STATE]\ntopic: null\ndepth: 0\nstatus: open" },
    { role: "user", content: "Please set the topic to incident" },
  ];
}

function turnEvents() {
  let n = seq;
  const next = () => n++;
  const out = [
    event(next(), "turn.started", { text: "Please set the topic to incident", userInput: "Please set the topic to incident" }),
    event(next(), "user.message", { text: "Please set the topic to incident" }),
    event(next(), "pass.started", { pass: "interpret", attempt: 1, model: "mock-small" }),
    event(next(), "prompt.rendered", {
      pass: "interpret",
      strategy: "cm.interpret.v1",
      hash: "interpret-hash",
      variables: { user_input: "Please set the topic to incident" },
      messageCount: 3,
    }),
    event(next(), "llm.request", {
      callId: "c-interpret",
      pass: "interpret",
      attempt: 1,
      round: 1,
      provider: "mock",
      purpose: "converse",
      model: "mock-small",
      messageCount: 3,
      toolCount: 0,
      messages: messages("interpret"),
      tools: [],
    }),
    event(next(), "llm.response", {
      callId: "c-interpret",
      status: "ok",
      model: "mock-small",
      text: "Register a check for the topic.",
      toolCalls: [],
      usage: { tokensIn: 10, tokensOut: 5 },
      latencyMs: 20,
      finishReason: "stop",
    }),
    event(next(), "pass.completed", {
      pass: "interpret",
      attempt: 1,
      finished: "action",
      text: "Register a check for the topic.",
      checks: ["topic is incident"],
      usage: { tokensIn: 10, tokensOut: 5 },
      durationMs: 25,
    }),
    event(next(), "verification.registered", { checks: [{ label: "topic", expectation: "incident" }] }),
    event(next(), "pass.started", { pass: "process", attempt: 1, model: "mock-small" }),
    event(next(), "prompt.rendered", {
      pass: "process",
      strategy: "cm.process.v1",
      hash: "process-hash",
      variables: { user_input: "Please set the topic to incident", retry_context: "" },
      messageCount: 3,
    }),
    event(next(), "llm.request", {
      callId: "c-process",
      pass: "process",
      attempt: 1,
      round: 1,
      provider: "mock",
      purpose: "converse",
      model: "mock-small",
      messageCount: 3,
      toolCount: tools.length,
      messages: messages("process"),
      tools,
    }),
    event(next(), "llm.response", {
      callId: "c-process",
      status: "ok",
      model: "mock-small",
      text: "Setting the topic.",
      toolCalls: [{ id: "tc-1", name: "conversation.set_topic", arguments: '{"topic":"incident"}' }],
      usage: { tokensIn: 30, tokensOut: 12 },
      latencyMs: 35,
      finishReason: "tool_calls",
    }),
    event(next(), "tool.call", {
      callId: "tc-1",
      name: "conversation.set_topic",
      arguments: '{"topic":"incident"}',
      pass: "process",
      attempt: 1,
      round: 1,
    }),
    event(next(), "tool.result", {
      callId: "tc-1",
      name: "conversation.set_topic",
      ok: true,
      output: '{"ok":true,"staged":true}',
      durationMs: 2,
      staged: true,
    }),
    event(next(), "internal.action", {
      pass: "process",
      component: "pipeline",
      action: "state.stage",
      ok: true,
      durationMs: 1,
    }),
    event(next(), "state.updated", { revision: 1, before: { topic: null }, after: { topic: "incident" } }),
    event(next(), "pass.completed", {
      pass: "process",
      attempt: 1,
      finished: "tools",
      text: "Setting the topic.",
      checks: [],
      usage: { tokensIn: 30, tokensOut: 12 },
      durationMs: 40,
    }),
    event(next(), "pass.started", { pass: "verify", attempt: 1, model: "mock-judge" }),
    event(next(), "verification.result", {
      label: "topic",
      backend: "llm_judge",
      pass: true,
      confidence: 0.94,
      reasoning: "The staged state sets topic to incident.",
      durationMs: 8,
      attempt: 1,
    }),
    event(next(), "pass.completed", {
      pass: "verify",
      attempt: 1,
      finished: "approved",
      text: null,
      checks: ["topic"],
      usage: { tokensIn: 0, tokensOut: 0 },
      durationMs: 8,
    }),
    event(next(), "agent.message", { text: "The topic is now incident.", final: true, verified: true, replyTo: "t-demo" }),
    event(next(), "turn.completed", {
      reply: "The topic is now incident.",
      model: "mock-small",
      attempts: 1,
      verified: true,
      usage: { tokensIn: 40, tokensOut: 17 },
      durationMs: 80,
    }),
  ];
  seq = n;
  return out;
}

function json(response, status, body) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

function sseFrame(response, record) {
  response.write(`id: ${record.seq}\nevent: ${record.type}\ndata: ${JSON.stringify(record)}\n\n`);
}

function broadcast(record) {
  for (const response of clients) sseFrame(response, record);
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);

  if (request.method === "GET" && url.pathname === "/api/health") {
    json(response, 200, {
      status: "ok",
      version: "mock",
      dataDir: "/tmp/mm-mock",
      sessions: 1,
      turnsStarted: 1,
      turnsCompleted: 1,
      turnsFailed: 0,
    });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/sessions") {
    json(response, 200, {
      sessions: [
        {
          sessionId,
          userId: "operator",
          title: "Demo session",
          createdAt: Date.now() - 1_000,
          lastSeq: seq - 1,
          updatedAt: Date.now(),
        },
      ],
    });
    return;
  }

  if (request.method === "GET" && url.pathname === `/api/agentstream/sessions/${sessionId}/events`) {
    const from = Number(url.searchParams.get("from") ?? "1");
    const limit = Number(url.searchParams.get("limit") ?? "500");
    const events = stored.filter((record) => record.seq >= from).slice(0, limit);
    json(response, 200, {
      sessionId,
      from,
      next: events.length ? (events.at(-1)?.seq ?? from) + 1 : from,
      events,
    });
    return;
  }

  if (request.method === "GET" && url.pathname === `/api/agentstream/sessions/${sessionId}/stream`) {
    const from = Number(url.searchParams.get("from") ?? "1");
    response.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    });
    response.write(": connected\n\n");
    for (const record of stored.filter((entry) => entry.seq >= from)) sseFrame(response, record);
    clients.add(response);
    request.on("close", () => clients.delete(response));
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/turns") {
    let body = "";
    request.on("data", (chunk) => {
      body += chunk;
    });
    request.on("end", () => {
      let text = "Please set the topic to incident";
      try {
        text = JSON.parse(body).text ?? text;
      } catch {
        // Keep the default prompt.
      }
      json(response, 202, { sessionId, turnId: "t-demo", seq: seq });
      const events = turnEvents();
      for (const record of events) {
        stored.push(record);
        setTimeout(() => broadcast(record), 15);
      }
      void text;
    });
    return;
  }

  json(response, 404, { message: "not found" });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`mock mm-serve on http://127.0.0.1:${port}`);
});
