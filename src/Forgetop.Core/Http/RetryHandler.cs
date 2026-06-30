using System.Net;

namespace Forgetop.Core.Http;

/// <summary>
/// Retries transient HTTP failures (5xx, 408, 429, network errors) with a short
/// linear backoff. Idempotent provider reads benefit; writes are typically retried
/// only on connection-level failures.
/// </summary>
public sealed class RetryHandler : DelegatingHandler
{
    private readonly int _maxRetries;
    private readonly TimeSpan _delay;

    public RetryHandler(HttpMessageHandler inner, int maxRetries = 2, TimeSpan? delay = null)
        : base(inner)
    {
        _maxRetries = maxRetries;
        _delay = delay ?? TimeSpan.FromMilliseconds(250);
    }

    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken ct)
    {
        for (var attempt = 0; ; attempt++)
        {
            try
            {
                var response = await base.SendAsync(request, ct).ConfigureAwait(false);
                if (attempt >= _maxRetries || !IsTransient(response.StatusCode))
                {
                    return response;
                }

                response.Dispose();
            }
            catch (HttpRequestException) when (attempt < _maxRetries)
            {
                // fall through to delay + retry
            }

            await Task.Delay(_delay * (attempt + 1), ct).ConfigureAwait(false);
        }
    }

    private static bool IsTransient(HttpStatusCode status) =>
        (int)status >= 500 || status == HttpStatusCode.RequestTimeout || status == HttpStatusCode.TooManyRequests;
}
