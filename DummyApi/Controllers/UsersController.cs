using Microsoft.AspNetCore.Mvc;
using DummyApi.Models;
using DummyApi.Services;

namespace DummyApi.Controllers;

[ApiController]
[Route("api/[controller]")]
public class UsersController : ControllerBase
{
    private readonly IUserService _userService;

    public UsersController(IUserService userService)
    {
        _userService = userService;
    }

    [HttpGet]
    public IActionResult GetAll() => Ok(_userService.GetAll());

    [HttpGet("{id}")]
    public IActionResult GetById(int id)
    {
        var user = _userService.GetById(id);
        return user is null ? NotFound() : Ok(user);
    }

    [HttpPost]
    public IActionResult Create([FromBody] CreateUserRequest request)
    {
        var user = _userService.Create(request);
        return CreatedAtAction(nameof(GetById), new { id = user.Id }, user);
    }

    [HttpPut("{id}")]
    public IActionResult Update(int id, [FromBody] UpdateUserRequest request)
    {
        var user = _userService.Update(id, request);
        return user is null ? NotFound() : Ok(user);
    }

    [HttpDelete("{id}")]
    public IActionResult Delete(int id)
    {
        var deleted = _userService.Delete(id);
        return deleted ? NoContent() : NotFound();
    }
}
