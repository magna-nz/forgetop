using System.Net;
using Forgetop.Core.Http;

namespace Forgetop.Core.Tests.Http;

public class RetryHandlerTests
{
    private sealed class StubHandler : HttpMessageHandler
    {
        private readonly Queue<Func<HttpResponseMessage>> _responses;

        public StubHandler(params Func<HttpResponseMessage>[] responses) => _responses = new(responses);

        public int Calls { get; private set; }

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken ct)
        {
            Calls++;
            return Task.FromResult(_responses.Dequeue()());
        }
    }

    private static HttpClient Client(StubHandler stub) =>
        new(new RetryHandler(stub, maxRetries: 2, delay: TimeSpan.Zero));

    [Fact]
    public async Task Retries_transient_5xx_then_succeeds()
    {
        var stub = new StubHandler(
            () => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable),
            () => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable),
            () => new HttpResponseMessage(HttpStatusCode.OK));

        var response = await Client(stub).GetAsync("http://example.test");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal(3, stub.Calls);
    }

    [Fact]
    public async Task Gives_up_after_max_retries()
    {
        var stub = new StubHandler(
            () => new HttpResponseMessage(HttpStatusCode.BadGateway),
            () => new HttpResponseMessage(HttpStatusCode.BadGateway),
            () => new HttpResponseMessage(HttpStatusCode.BadGateway));

        var response = await Client(stub).GetAsync("http://example.test");

        Assert.Equal(HttpStatusCode.BadGateway, response.StatusCode);
        Assert.Equal(3, stub.Calls); // initial + 2 retries
    }

    [Fact]
    public async Task Does_not_retry_4xx()
    {
        var stub = new StubHandler(() => new HttpResponseMessage(HttpStatusCode.NotFound));

        var response = await Client(stub).GetAsync("http://example.test");

        Assert.Equal(HttpStatusCode.NotFound, response.StatusCode);
        Assert.Equal(1, stub.Calls);
    }
}
