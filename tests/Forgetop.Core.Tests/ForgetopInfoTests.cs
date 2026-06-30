using Forgetop.Core;

namespace Forgetop.Core.Tests;

public class ForgetopInfoTests
{
    [Fact]
    public void Name_is_forgetop()
    {
        Assert.Equal("forgetop", ForgetopInfo.Name);
    }
}
