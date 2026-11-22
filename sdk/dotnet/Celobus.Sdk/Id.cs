using System.Globalization;

namespace Celobus.Sdk
{
    /// <summary>
    /// The key of an entity <typeparamref name="T"/>, or a ref to it (<c>Id&lt;Platform.User&gt;</c> for a person of the
    /// workspace). It wraps the key in its wire form: a UUID, an int64 or a string, all JSON strings. An id of one
    /// entity cannot be passed where another's is expected.
    /// </summary>
    public readonly struct Id<T> : IEquatable<Id<T>> where T : class
    {
        private readonly string? _value;

        public Id(string value)
        {
            ArgumentException.ThrowIfNullOrEmpty(value);
            _value = value;
        }

        public Id(long value) : this(value.ToString(CultureInfo.InvariantCulture)) { }

        public Id(Guid value) : this(value.ToString()) { }

        /// <summary>The key as it travels (empty for <c>default</c>).</summary>
        public string Value => _value ?? "";

        public bool IsEmpty => string.IsNullOrEmpty(_value);

        /// <summary>The key of an <c>i64</c> entity.</summary>
        public long ToInt64() => long.Parse(Value, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);

        /// <summary>The key of a <c>uuid</c> entity.</summary>
        public Guid ToGuid() => Guid.Parse(Value);

        public bool Equals(Id<T> other) => string.Equals(Value, other.Value, StringComparison.Ordinal);
        public override bool Equals(object? obj) => obj is Id<T> other && Equals(other);
        public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Value);
        public static bool operator ==(Id<T> a, Id<T> b) => a.Equals(b);
        public static bool operator !=(Id<T> a, Id<T> b) => !a.Equals(b);
        public override string ToString() => Value;
    }
}

namespace Celobus.Sdk.Platform
{
    /// <summary>
    /// A person (or service account) of the workspace directory: <c>Id&lt;Platform.User&gt;</c> is <c>ref:platform.user</c>.
    /// Its properties are the columns a query names (<c>ctx.Query&lt;Platform.User&gt;()</c>); the SDK never fills them.
    /// <see cref="Email"/> is read only with the plugin permission <c>members.email</c> and never by a guest.
    /// </summary>
    [RefTarget("platform.user")]
    public sealed class User
    {
        private User() { }

        public Id<User> Id { get; private init; }
        public string DisplayName { get; private init; } = "";
        public string AvatarUrl { get; private init; } = "";
        public string? Email { get; private init; }
        public UserKind Kind { get; private init; }
        /// <summary>The base role in the workspace; null for a person who left.</summary>
        public BaseRole? BaseRole { get; private init; }
        public UserStatus Status { get; private init; }
        public string Locale { get; private init; } = "";
        public DateTimeOffset JoinedAt { get; private init; }
        public DateTimeOffset? LeftAt { get; private init; }
    }

    /// <summary>A group of the workspace directory: <c>Id&lt;Platform.Group&gt;</c> is <c>ref:platform.group</c>.</summary>
    [RefTarget("platform.group")]
    public sealed class Group
    {
        private Group() { }

        public Id<Group> Id { get; private init; }
        public string Name { get; private init; } = "";
    }

    /// <summary>A person's membership of a group (<c>platform.group_member</c>), for queries.</summary>
    public sealed class GroupMember
    {
        private GroupMember() { }

        public Id<Group> GroupId { get; private init; }
        public Id<User> UserId { get; private init; }
    }

    /// <summary>A role a person holds (<c>platform.role_assignment</c>: <c>workspace.*</c> and the plugin's own), for queries.</summary>
    public sealed class RoleAssignment
    {
        private RoleAssignment() { }

        public Id<User> UserId { get; private init; }
        public string Role { get; private init; } = "";
    }

    /// <summary>A person or a service account.</summary>
    public enum UserKind { Person, Service }

    /// <summary>The base role in the workspace.</summary>
    public enum BaseRole { Owner, Admin, Member, Guest }

    /// <summary>Whether a person is in the workspace.</summary>
    public enum UserStatus { Active, Removed, Erased }
}
