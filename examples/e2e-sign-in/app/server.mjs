// Adapted from e2e's Apache-2.0 testbed. This example keeps only the sign-in
// routes and uses revocable, in-memory sessions. See NOTICE.
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:http';

const port = Number(process.env.PORT ?? 4271);
const sessions = new Set();
const cookieName = 'moli_demo_session';
const cookieFlags = 'Path=/; HttpOnly; SameSite=Lax';

function page(title, content) {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${title}</title>
  <style>
    body { margin: 0; color: #192e43; background: #f1f5f8; font: 18px/1.5 system-ui, sans-serif; }
    main { max-width: 32rem; margin: 4rem auto; padding: 2rem; background: white; border-top: 4px solid #176954; }
    h1 { margin: 0 0 1rem; font-size: 2rem; }
    label, input { display: block; }
    input { box-sizing: border-box; width: 100%; padding: .5rem; margin: .25rem 0 1rem; }
    button { padding: .5rem 1rem; font: inherit; color: white; background: #176954; border: 0; }
    [role=alert] { color: #9d332c; }
  </style>
</head>
<body><main><h1>${title}</h1>${content}</main></body>
</html>`;
}

function signInForm(failed = false) {
  return page('Sign in', `
    ${failed ? '<p role="alert">Invalid credentials</p>' : ''}
    <form method="post" action="/login">
      <label for="username">Username</label>
      <input id="username" name="username" autocomplete="username" required>
      <label for="password">Password</label>
      <input id="password" name="password" type="password" autocomplete="current-password" required>
      <button type="submit">Sign in</button>
    </form>`);
}

const server = createServer(async (request, response) => {
  const { pathname } = new URL(request.url, 'http://127.0.0.1');
  const sessionId = (request.headers.cookie ?? '').split(';')
    .map(cookie => cookie.trim())
    .find(cookie => cookie.startsWith(`${cookieName}=`))?.slice(cookieName.length + 1);

  response.setHeader('Cache-Control', 'no-store');
  function redirect(location) {
    response.writeHead(303, { Location: location });
    response.end();
  }
  function html(content) {
    response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    response.end(content);
  }

  if (request.method === 'GET' && (pathname === '/' || pathname === '/login')) {
    return html(signInForm());
  }
  if (request.method === 'POST' && pathname === '/login') {
    let body = '';
    for await (const chunk of request) body += chunk;
    const form = new URLSearchParams(body);
    if (form.get('username') !== 'admin' || form.get('password') !== 'admin-pass') {
      return html(signInForm(true));
    }
    const id = randomUUID();
    sessions.add(id);
    response.setHeader('Set-Cookie', `${cookieName}=${id}; ${cookieFlags}`);
    return redirect('/dashboard');
  }
  if (request.method === 'GET' && pathname === '/dashboard') {
    if (!sessions.has(sessionId)) return redirect('/login');
    return html(page('Dashboard', `
      <p role="status" aria-label="Greeting">Welcome back, admin!</p>
      <form method="post" action="/logout"><button type="submit">Sign out</button></form>`));
  }
  if (request.method === 'POST' && pathname === '/logout') {
    sessions.delete(sessionId);
    response.setHeader('Set-Cookie', `${cookieName}=; ${cookieFlags}; Max-Age=0`);
    return redirect('/login');
  }
  response.writeHead(404);
  response.end('Page not found');
});

server.listen(port, '127.0.0.1', () => {
  console.log(`Sign-in demo at http://127.0.0.1:${server.address().port}`);
});
