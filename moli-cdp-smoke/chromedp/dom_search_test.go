package chromedp_smoke

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/chromedp"
)

func startMoli(t *testing.T, ctx context.Context) string {
	t.Helper()
	binary := os.Getenv("MOLI_BIN")
	if binary == "" {
		binary = "../../target/debug/moli"
	}
	binary, err := filepath.Abs(binary)
	if err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().(*net.TCPAddr)
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	var logs bytes.Buffer
	cmd := exec.Command(binary, "serve", "--host", "127.0.0.1", "--port", fmt.Sprint(address.Port))
	cmd.Stdout, cmd.Stderr = &logs, &logs
	cmd.Env = append(os.Environ(), "HTTP_PROXY=", "HTTPS_PROXY=", "ALL_PROXY=", "http_proxy=", "https_proxy=", "all_proxy=")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		if t.Failed() {
			t.Logf("Moli output:\n%s", logs.String())
		}
	})
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	t.Cleanup(client.CloseIdleConnections)
	ticker := time.NewTicker(25 * time.Millisecond)
	defer ticker.Stop()
	for {
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, "http://"+address.String()+"/json/version", nil)
		if err != nil {
			t.Fatal(err)
		}
		response, err := client.Do(request)
		if err == nil {
			var version struct {
				WebSocketURL string `json:"webSocketDebuggerUrl"`
			}
			err = json.NewDecoder(response.Body).Decode(&version)
			response.Body.Close()
			if err == nil && version.WebSocketURL != "" {
				return version.WebSocketURL
			}
		}
		select {
		case <-ctx.Done():
			t.Fatalf("Moli discovery: %v", ctx.Err())
		case <-ticker.C:
		}
	}
}

func TestDOMSearchPublishesNodePaths(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
	endpoint := startMoli(t, ctx)
	fixture := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		fmt.Fprint(w, "<!doctype html><html><body><main><section><ul id='links'>")
		if r.URL.Path == "/static" {
			for i := range 12 {
				fmt.Fprintf(w, "<li><a href='/next/%d'>link %d</a></li>", i, i)
			}
		}
		fmt.Fprint(w, "</ul></section></main>")
		if r.URL.Path == "/trees" {
			fmt.Fprint(w, `<main><section><a href="/light">light</a></section></main>
<div id="host"></div><iframe srcdoc="<main><a href='/frame'>frame</a></main>"></iframe>
<script>document.getElementById('host').attachShadow({mode:'closed'}).innerHTML='<section><a href="/shadow">shadow</a></section>';</script>`)
		}
		if r.URL.Path == "/script" {
			fmt.Fprint(w, `<script>
for (let i = 0; i < 12; i++) {
  const li = document.createElement('li');
  const a = document.createElement('a');
  a.href = '/next/' + i;
  a.textContent = 'link ' + i;
  li.append(a);
  document.getElementById('links').append(li);
}

</script>`)
		}
		fmt.Fprint(w, "</body></html>")
	}))
	defer fixture.Close()
	allocator, cancelAllocator := chromedp.NewRemoteAllocator(ctx, endpoint, chromedp.NoModifyURL)
	defer cancelAllocator()
	browser, cancelBrowser := chromedp.NewContext(allocator, chromedp.WithDebugf(t.Logf))
	defer cancelBrowser()
	var links []string
	for i := range 12 {
		links = append(links, fmt.Sprintf("/next/%d", i))
	}
	for _, fixtureCase := range []struct {
		path  string
		hrefs []string
	}{
		{"/static", links},
		{"/script", links},
		{"/trees", []string{"/light", "/shadow", "/frame"}},
	} {
		t.Run(fixtureCase.path, func(t *testing.T) {
			if err := chromedp.Run(browser, chromedp.Navigate(fixture.URL+fixtureCase.path)); err != nil {
				t.Fatal(err)
			}
			for range 2 {
				var nodes []*cdp.Node
				// The default BySearch / NodeReady path uses the frontend node cache.
				// Do not replace this with ByJSPath or Runtime.evaluate.
				if err := chromedp.Run(browser, chromedp.Nodes("a[href]", &nodes, chromedp.AtLeast(1))); err != nil {
					t.Fatalf("get links: %v", err)
				}
				if len(nodes) != len(fixtureCase.hrefs) {
					t.Fatalf("got %d links, want %d", len(nodes), len(fixtureCase.hrefs))
				}
				for i, node := range nodes {
					if got, want := node.AttributeValue("href"), fixtureCase.hrefs[i]; got != want {
						t.Fatalf("link %d: href=%q, want %q", i, got, want)
					}
				}
			}
		})
	}
}

func TestDOMSearchPublicationAcrossCommandsAndDynamicTrees(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 60*time.Second)
	defer cancel()
	endpoint := startMoli(t, ctx)
	fixture := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		fmt.Fprint(w, `<!doctype html><html><body>
<main><section><a id="target" href="/next">link</a></section></main>
<div id="host"></div><iframe id="frame" srcdoc="<p>original</p>"></iframe>
</body></html>`)
	}))
	defer fixture.Close()
	allocator, cancelAllocator := chromedp.NewRemoteAllocator(ctx, endpoint, chromedp.NoModifyURL)
	defer cancelAllocator()
	for _, test := range []struct {
		name, expression, selector, command string
	}{
		{"describe-target", `document.getElementById('target')`, "#target", "describe"},
		{"describe-ancestor", `document.querySelector('section')`, "#target", "describe"},
		{"request-target", `document.getElementById('target')`, "#target", "request"},
		{"request-ancestor", `document.querySelector('section')`, "#target", "request"},
		{"new-shadow-root", `document.getElementById('host').attachShadow({mode:'closed'}).innerHTML='<section><a id=shadow-hit href=/shadow>shadow</a></section>'`, "#shadow-hit", "evaluate"},
		{"replacement-frame-document", `new Promise(resolve => {const f=document.getElementById('frame');f.onload=()=>resolve(true);f.srcdoc='<main><a id=frame-hit href=/frame>replacement</a></main>';})`, "#frame-hit", "evaluate"},
	} {
		t.Run(test.name, func(t *testing.T) {
			browser, cancelBrowser := chromedp.NewContext(allocator, chromedp.WithDebugf(t.Logf))
			defer cancelBrowser()
			query, cancelQuery := context.WithTimeout(browser, 10*time.Second)
			defer cancelQuery()
			var owners []*cdp.Node
			if err := chromedp.Run(query,
				chromedp.Navigate(fixture.URL),
				// Publish the owners before changing their associated trees.
				chromedp.Nodes("#host, #frame", &owners, chromedp.ByQueryAll, chromedp.AtLeast(2)),
				chromedp.ActionFunc(func(ctx context.Context) error {
					object, exception, err := runtime.Evaluate(test.expression).WithAwaitPromise(true).Do(ctx)
					if err != nil {
						return err
					}
					if exception != nil {
						return fmt.Errorf("evaluate: %v", exception)
					}
					switch test.command {
					case "describe":
						_, err = dom.DescribeNode().WithObjectID(object.ObjectID).Do(ctx)
					case "request":
						_, err = dom.RequestNode(object.ObjectID).Do(ctx)
					}
					return err
				}),
			); err != nil {
				t.Fatal(err)
			}
			for range 2 {
				var nodes []*cdp.Node
				if err := chromedp.Run(query, chromedp.Nodes(test.selector, &nodes)); err != nil {
					t.Fatalf("BySearch / NodeReady: %v", err)
				}
				if len(nodes) != 1 || nodes[0].AttributeValue("href") == "" {
					t.Fatalf("expected one usable link in the frontend cache, got %v", nodes)
				}
			}
		})
	}
}
