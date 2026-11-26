# Making a sandbox over SSH

Connecting over SSH as `<sandbox>.<handle>` to a sandbox you do not have yet makes it. How it is
made depends on what the connection asks for.

## On a terminal: the installer

`ssh -p 48291 new.<handle>@<host>`, a shell on a terminal, opens an installer in the terminal
before any sandbox exists:

1. **Name**, only when connecting as just `<handle>` with no sandbox at all: the new sandbox's
   name.
2. **Image**: one of the catalog's images, or, where the admin lets users name images, **Another
   image**, then its reference. The reference is checked at once against the registries allowed
   and the most an image may take to download, and a problem shows under it.
3. **Size**: one of the catalog's sizes, with its processors, memory and home.
4. **Setup**: what the sandbox comes with, by name and kind (variable or file, template, secret),
   values never shown. The admin's **required** entries come first, fixed: every sandbox gets
   them. The **offered** shared entries follow, then your own; those that apply to new sandboxes
   come checked, the others unchecked.
5. **Repository**: a git repository the first start clones into the home, by its URL, then its
   branch, left empty for the repository's default. An empty URL clones nothing and skips the
   branch. A URL or branch Corral would not clone, such as a URL with a login in it, shows why
   under it.
6. **Summary**, then **Create**: the installer counts the seconds while the sandbox is created
   and started, then the sandbox's shell opens on the same connection.

Each step starts from your **Account** defaults. Keys: Up and Down choose, Space checks an entry,
Enter goes on, Left goes back, and Esc or q leaves with nothing made. In a field you type in, Up
goes back and only Esc leaves. A creation Corral refuses, such as one past your sandbox limit, or
a template whose variable is not chosen, shows why, and Enter goes back to the summary to change
the choices.

The installer fits a terminal of 80 by 24 and needs no color: with `NO_COLOR` set (for example
`ssh -o SetEnv=NO_COLOR=1 ...`) it draws without colors. On a terminal that cannot draw a full
screen (`TERM=dumb`) it asks the same questions on lines, each answered with a number, a list of
numbers, or Enter to keep what is shown. The repository's URL and branch are typed in full, and
Enter alone means none, or the default branch.

## From a script: `corral-new`

`ssh new.<handle>@<host> corral-new` reads a JSON spec on its input, makes the sandbox, waits until
it takes sessions, and prints what it made as JSON:

```sh
echo '{"image": "dev", "size": "large", "setup": ["GH_TOKEN", ".gitconfig"]}' \
  | ssh -p 48291 new.<handle>@<host> corral-new
```

```json
{"name":"new","sshUsername":"new.adam","image":"dev","size":"large","setup":[{"id":"…","kind":"variable","name":"GH_TOKEN"},{"id":"…","kind":"file","name":".gitconfig"}],"repository":null}
```

Every field of the spec is optional, and empty input is the empty spec `{}`:

| Field | Value | Left out |
| --- | --- | --- |
| `image` | a catalog image's key, or else an image reference, where users may name images | your default image |
| `size` | a catalog size's key | your default size |
| `setup` | entries to attach, each by its id or by the variable or file it sets (`GH_TOKEN`, `.gitconfig`) | the entries that apply to new sandboxes |
| `repository` | a git repository the first start clones into the home, `{"url": "...", "branch": "..."}`, the branch optional | no repository |

`"setup": []` attaches none, and the admin's required entries reach the sandbox whatever `setup`
lists. A name that both your own entry and a shared one set picks yours. A repository URL is
`https://...` without a login, `ssh://...` without a password, `file:///...` or `user@host:path`:
a token goes in Setup, as a git token or SSH key. An unknown field is refused. On a refusal, such
as a bad spec, an unknown size, or a sandbox by that name that exists already, nothing is made: it
prints `{"error": "..."}` on its error output and exits with status 1.

## Any other command

Any other command to a sandbox that does not exist yet, such as `hc machine add` pointing at a
new name, or a shell without a terminal, makes the sandbox from your Account defaults with the
entries that apply to new sandboxes, then runs.
