import { CoApiClient, CoApiError } from "../src/api";

const BASE_URL = "https://co.example.com";
const TOKEN = "co_testtoken";
const SLUG = "my-universe";

function mockFetch(
  status: number,
  body: unknown,
  contentType = "application/json",
) {
  return jest.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? "OK" : "Error",
    headers: { get: (_: string) => contentType },
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(typeof body === "string" ? body : JSON.stringify(body)),
  });
}

let client: CoApiClient;

beforeEach(() => {
  client = new CoApiClient(BASE_URL, TOKEN);
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("CoApiClient.listFiles", () => {
  it("calls the correct vault list URL", async () => {
    const fetchMock = mockFetch(200, { files: [] });
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.listFiles(SLUG);

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/universes/${SLUG}/vault/`,
      expect.objectContaining({
        method: "GET",
        headers: expect.objectContaining({
          Authorization: `Bearer ${TOKEN}`,
        }),
      }),
    );
  });

  it("returns the files array", async () => {
    const files = [
      { path: "projects/CO/1.md", stat: { ctime: 100, mtime: 200, size: 300 } },
    ];
    global.fetch = mockFetch(200, { files }) as unknown as typeof fetch;

    const result = await client.listFiles(SLUG);
    expect(result).toEqual(files);
  });

  it("throws CoApiError on 401", async () => {
    global.fetch = mockFetch(401, "Unauthorized") as unknown as typeof fetch;
    await expect(client.listFiles(SLUG)).rejects.toBeInstanceOf(CoApiError);
  });
});

describe("CoApiClient.getFile", () => {
  it("calls the correct vault file URL", async () => {
    const fetchMock = mockFetch(200, {
      path: "notes/foo.md",
      content: "# Hello",
      frontmatter: {},
      tags: [],
      stat: { ctime: 0, mtime: 0, size: 7 },
    });
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.getFile(SLUG, "notes/foo.md");

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/universes/${SLUG}/vault/notes/foo.md`,
      expect.objectContaining({ method: "GET" }),
    );
  });

  it("strips leading slash from file path", async () => {
    const fetchMock = mockFetch(200, {
      path: "notes/foo.md",
      content: "",
      frontmatter: {},
      tags: [],
      stat: { ctime: 0, mtime: 0, size: 0 },
    });
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.getFile(SLUG, "/notes/foo.md");

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/universes/${SLUG}/vault/notes/foo.md`,
      expect.anything(),
    );
  });
});

describe("CoApiClient.putFile", () => {
  it("sends PUT with text/markdown content-type", async () => {
    const fetchMock = mockFetch(200, "", "text/plain");
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.putFile(SLUG, "notes/new.md", "# New note");

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/universes/${SLUG}/vault/notes/new.md`,
      expect.objectContaining({
        method: "PUT",
        headers: expect.objectContaining({
          "Content-Type": "text/markdown",
          Authorization: `Bearer ${TOKEN}`,
        }),
        body: "# New note",
      }),
    );
  });
});

describe("CoApiClient.deleteFile", () => {
  it("sends DELETE request", async () => {
    const fetchMock = mockFetch(204, "");
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.deleteFile(SLUG, "notes/old.md");

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/universes/${SLUG}/vault/notes/old.md`,
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("adds ?permanent=true when requested", async () => {
    const fetchMock = mockFetch(204, "");
    global.fetch = fetchMock as unknown as typeof fetch;

    await client.deleteFile(SLUG, "notes/old.md", true);

    const [url] = fetchMock.mock.calls[0] as [string, unknown];
    expect(url).toContain("?permanent=true");
  });
});

describe("CoApiClient.getMyUniverses", () => {
  it("calls /api/v1/me/universes", async () => {
    const resp = {
      owned: [{ key: "my-u", name: "My Universe", visibility: "private" }],
      member: [],
      subscribed: [],
      counts: { owned: 1, member: 0, subscribed: 0, invited: 0, discoverable: 0 },
    };
    const fetchMock = mockFetch(200, resp);
    global.fetch = fetchMock as unknown as typeof fetch;

    const result = await client.getMyUniverses();

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/v1/me/universes`,
      expect.anything(),
    );
    expect(result.owned[0]?.key).toBe("my-u");
  });
});
