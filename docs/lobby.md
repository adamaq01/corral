# Making a sandbox over SSH

Connecting over SSH as `<sandbox>.<handle>` to a sandbox you do not have yet makes it. How it is
made depends on what the connection asks for.

## On a terminal: the installer

`ssh -p 48291 new.<handle>@<host>`, a shell on a terminal, opens an installer in the terminal
before any sandbox exists:

1. **Image**: one of the catalog's images, or, where the admin lets users name images, **Another
   image**, then its reference. The reference is checked at once against the registries allowed
   and the most an image may take to download, and a problem shows under it.
2. **Size**: one of the catalog's sizes, with its processors, memory and home.
3. **Environment**: your variables and files, by name and kind (variable or file, template,
   secret). Those marked **Apply to new sandboxes** come checked; the others are listed
   unchecked. Values never show.
4. **Summary**, then **Create**: the installer counts the seconds while the sandbox is created
   and started, then the sandbox's shell opens on the same connection.

Each step starts from your **Account** defaults. Keys: Up and Down choose, Space checks an entry,
Enter goes on, Left goes back, and Esc or q leaves with nothing made. A creation Corral refuses,
such as one past your sandbox limit, or a template whose variable is not chosen, shows why, and
Enter goes back to the summary to change the choices.

Connecting as just `<handle>` when you have no sandbox at all asks for the new sandbox's name
first.

The installer fits a terminal of 80 by 24 and needs no color: with `NO_COLOR` set (for example
`ssh -o SetEnv=NO_COLOR=1 ...`) it draws without colors. On a terminal that cannot draw a full
screen (`TERM=dumb`) it asks the same questions on lines, each answered with a number, a list of
numbers, or Enter to keep what is shown.

## From a script: `corral-new`

`ssh new.<handle>@<host> corral-new` reads a JSON spec on its input, makes the sandbox, waits until
it takes sessions, and prints what it made as JSON:

```sh
echo '{"image": "dev", "size": "large", "environment": ["GH_TOKEN", ".gitconfig"]}' \
  | ssh -p 48291 new.<handle>@<host> corral-new
```

```json
{"name":"new","sshUsername":"new.adam","image":"dev","size":"large","environment":[{"id":"…","kind":"variable","name":"GH_TOKEN"},{"id":"…","kind":"file","name":".gitconfig"}]}
```

Every field of the spec is optional, and empty input is the empty spec `{}`:

| Field | Value | Left out |
| --- | --- | --- |
| `image` | a catalog image's key, or else an image reference, where users may name images | your default image |
| `size` | a catalog size's key | your default size |
| `environment` | entries to attach, each by its id or by the variable or file it sets (`GH_TOKEN`, `.gitconfig`) | the entries that apply to new sandboxes |

`"environment": []` attaches none. An unknown field is refused. On a refusal, such as a bad spec,
an unknown size, or a sandbox by that name that exists already, nothing is made: it prints
`{"error": "..."}` on its error output and exits with status 1.

## Any other command

Any other command to a sandbox that does not exist yet, such as `hc machine add` pointing at a
new name, or a shell without a terminal, makes the sandbox from your Account defaults with the
entries that apply to new sandboxes, then runs.
