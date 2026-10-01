"""Drive ChakraMCP's built-in sign-in pages the way a browser does: cookies
kept between requests, forms posted with their hidden fields and an Origin
header, redirects followed. Standard library only.

Used by the end-to-end test (run.sh) through browser.py (the CLI's
$BROWSER) and mcp_client.py.
"""

import html.parser
import http.cookiejar
import urllib.error
import urllib.parse
import urllib.request


class Page:
    def __init__(self, url, status, body):
        self.url = url
        self.status = status
        self.body = body
        parser = _Parser()
        parser.feed(body)
        self.forms = parser.forms
        self.heading = parser.heading.strip()
        self.error = parser.error.strip()

    def form(self, action):
        """The form posting to `action` (a path)."""
        for form in self.forms:
            if form["action"] == action:
                return form
        raise AssertionError(f"no form for {action} on {self.url} ({self.heading!r})")

    def __repr__(self):
        return f"<{self.status} {self.url} {self.heading!r}{' error=' + repr(self.error) if self.error else ''}>"


class _Parser(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.forms = []
        self.heading = ""
        self.error = ""
        self._in = None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "form":
            self.forms.append({"action": a.get("action", ""), "method": a.get("method", "get"), "fields": []})
        elif tag == "input" and self.forms and a.get("type") == "hidden" and a.get("name"):
            self.forms[-1]["fields"].append((a["name"], a.get("value", "")))
        elif tag == "h1":
            self._in = "heading"
        elif tag == "p" and "error" in (a.get("class") or "").split():
            self._in = "error"

    def handle_endtag(self, tag):
        if tag in ("h1", "p"):
            self._in = None

    def handle_data(self, data):
        if self._in == "heading":
            self.heading += data
        elif self._in == "error":
            self.error += data


class Browser:
    def __init__(self, log=print):
        self.log = log
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )

    def _open(self, request):
        try:
            with self.opener.open(request, timeout=30) as response:
                return Page(response.geturl(), response.status, response.read().decode("utf-8", "replace"))
        except urllib.error.HTTPError as err:
            return Page(err.geturl(), err.code, err.read().decode("utf-8", "replace"))

    def get(self, url):
        page = self._open(urllib.request.Request(url))
        self.log(f"GET  {page}")
        return page

    def submit(self, page, action, **extra):
        """Post `page`'s form for `action` with its hidden fields plus `extra`."""
        form = page.form(action)
        fields = [(k, v) for k, v in form["fields"] if k not in extra]
        for key, value in extra.items():
            for v in value if isinstance(value, list) else [value]:
                fields.append((key, v))
        parts = urllib.parse.urlsplit(page.url)
        origin = f"{parts.scheme}://{parts.netloc}"
        request = urllib.request.Request(
            urllib.parse.urljoin(page.url, action),
            data=urllib.parse.urlencode(fields).encode(),
            headers={"Origin": origin, "Content-Type": "application/x-www-form-urlencoded"},
            method="POST",
        )
        result = self._open(request)
        self.log(f"POST {action} -> {result}")
        return result


def sign_in_if_asked(browser, page, action, email, password):
    """Sign in when `page` is the sign-in form; return the page after it."""
    if page.heading != "Sign in":
        return page
    page = browser.submit(page, action, step="signin", email=email, password=password)
    if page.heading == "Sign in":
        raise AssertionError(f"sign-in failed: {page.error or page.status}")
    return page


def authorize(browser, url, email, password, agent_scope="all"):
    """Sign in and approve on /oauth/authorize; return the client's callback page."""
    page = sign_in_if_asked(browser, browser.get(url), "/oauth/authorize", email, password)
    if "wants to use your account" not in page.heading:
        raise AssertionError(f"expected the consent page, got {page}")
    return browser.submit(page, "/oauth/authorize", step="approve", agent_scope=agent_scope)


def pair(browser, url, email, password, display_name, slug):
    """Sign in and approve a device pairing as a new agent."""
    page = sign_in_if_asked(browser, browser.get(url), "/app/pair", email, password)
    if page.heading != "Connect an agent":
        raise AssertionError(f"expected the pairing form, got {page}")
    page = browser.submit(
        page, "/app/pair", step="approve", target="new",
        display_name=display_name, slug=slug, visibility="private",
    )
    if page.heading != "Approved":
        raise AssertionError(f"pairing wasn't approved: {page}")
    return page
