using DummyApi.Models;

namespace DummyApi.Services;

public interface IUserService
{
    IEnumerable<User> GetAll();
    User? GetById(int id);
    User Create(CreateUserRequest request);
    User? Update(int id, UpdateUserRequest request);
    bool Delete(int id);
}

public class UserService : IUserService
{
    private readonly List<User> _users = new()
    {
        new User { Id = 1, Name = "Alice Smith", Email = "alice@example.com" },
        new User { Id = 2, Name = "Bob Jones", Email = "bob@example.com" },
        new User { Id = 3, Name = "Carol White", Email = "carol@example.com" },
    };

    private int _nextId = 4;

    public IEnumerable<User> GetAll() => _users;

    public User? GetById(int id) => _users.FirstOrDefault(u => u.Id == id);

    public User Create(CreateUserRequest request)
    {
        var user = new User
        {
            Id = _nextId++,
            Name = request.Name,
            Email = request.Email,
        };
        _users.Add(user);
        return user;
    }

    public User? Update(int id, UpdateUserRequest request)
    {
        var user = GetById(id);
        if (user is null) return null;

        if (request.Name is not null) user.Name = request.Name;
        if (request.Email is not null) user.Email = request.Email;
        if (request.IsActive is not null) user.IsActive = request.IsActive.Value;

        return user;
    }

    public bool Delete(int id)
    {
        var user = GetById(id);
        if (user is null) return false;
        _users.Remove(user);
        return true;
    }
}
