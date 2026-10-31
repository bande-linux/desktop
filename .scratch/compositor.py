p = 'crates/platform/compositor/src/alert.rs'
s = open(p).read()
a = '''            format!("Cannot start {}: {error}", executable.display()),'''
b = '''            crate::L10N.tr_args(
                "cannot-start",
                [
                    ("program", executable.display().to_string().into()),
                    ("error", error.to_string().into()),
                ],
            ),'''
assert a in s
s = s.replace(a, b)
open(p, 'w').write(s)
p = 'crates/platform/compositor/locales/en-US/unbound-compositor.ftl'
s = open(p).read()
s = s.rstrip('\n') + '\ncannot-start = { $program } can’t start: { $error }\n'
open(p, 'w').write(s)
