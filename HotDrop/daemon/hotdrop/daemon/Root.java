package hotdrop.daemon;

import java.nio.file.Path;

/**
 * A source tree and the directory its classes are written to.
 * {@code scope} bounds which loaded classes the agent may redefine (for Hybris, the extension directory).
 */
final class Root {
    final String name;
    final Path src;
    final Path out;
    final Path scope;
    final boolean web;

    Root(String name, Path src, Path out, Path scope, boolean web) {
        this.name = name;
        this.src = src.toAbsolutePath().normalize();
        this.out = out.toAbsolutePath().normalize();
        this.scope = scope.toAbsolutePath().normalize();
        this.web = web;
    }

    @Override
    public String toString() {
        return name;
    }
}
