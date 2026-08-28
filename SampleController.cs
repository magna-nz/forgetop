using Microsoft.AspNetCore.Mvc;

namespace SampleApp.Controllers;

[ApiController]
[Route("api/[controller]")]
public class SampleController : ControllerBase
{
    private static readonly List<string> Items = new() { "alpha", "beta", "gamma" };

    [HttpGet]
    public ActionResult<IEnumerable<string>> GetAll() => Ok(Items);

    [HttpGet("{id:int}")]
    public ActionResult<string> GetById(int id)
    {
        if (id < 0 || id >= Items.Count)
            return NotFound();

        return Ok(Items[id]);
    }

    [HttpPost]
    public ActionResult<string> Create([FromBody] string value)
    {
        if (string.IsNullOrWhiteSpace(value))
            return BadRequest("Value cannot be empty.");

        Items.Add(value);
        return CreatedAtAction(nameof(GetById), new { id = Items.Count - 1 }, value);
    }

    [HttpDelete("{id:int}")]
    public IActionResult Delete(int id)
    {
        if (id < 0 || id >= Items.Count)
            return NotFound();

        Items.RemoveAt(id);
        return NoContent();
    }
}
