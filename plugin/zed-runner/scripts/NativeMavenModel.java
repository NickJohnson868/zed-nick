import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Properties;
import org.apache.maven.artifact.Artifact;
import org.apache.maven.eventspy.AbstractEventSpy;
import org.apache.maven.execution.ExecutionEvent;
import org.apache.maven.project.MavenProject;
import org.apache.maven.project.DefaultDependencyResolutionRequest;
import org.apache.maven.project.ProjectDependenciesResolver;
import org.eclipse.aether.util.filter.DependencyFilterUtils;

/** Exports Maven's resolved reactor model without resolving packaged reactor artifacts. */
public final class NativeMavenModel extends AbstractEventSpy {
    @Override
    public void onEvent(Object event) throws Exception {
        if (!(event instanceof ExecutionEvent execution) || execution.getType() != ExecutionEvent.Type.SessionEnded) return;
        var session = execution.getSession();
        if (session.getResult().hasExceptions()) return;
        String module = session.getUserProperties().getProperty("zed.runner.module");
        String destination = session.getUserProperties().getProperty("zed.runner.model");
        if (module == null || destination == null) return;
        var projects = new LinkedHashMap<String, MavenProject>();
        MavenProject selected = null;
        for (MavenProject project : session.getProjects()) {
            projects.put(key(project.getArtifact()), project);
            if (project.getBasedir().toPath().toRealPath().equals(Path.of(module).toRealPath())) selected = project;
        }
        if (Boolean.parseBoolean(session.getUserProperties().getProperty("zed.runner.discover"))) {
            var model = new Properties();
            model.setProperty("contains.module", Boolean.toString(selected != null));
            try (var stream = Files.newOutputStream(Path.of(destination))) {
                model.store(stream, "Maven resolved reactor membership");
            }
            return;
        }
        if (selected == null) throw new IllegalStateException("Startup module is absent from Maven reactor");
        var classpath = new LinkedHashSet<Path>();
        var modules = new LinkedHashSet<Path>();
        Path output = Path.of(selected.getBuild().getOutputDirectory()).toRealPath();
        classpath.add(output);
        modules.add(selected.getBasedir().toPath().toRealPath());
        var request = new DefaultDependencyResolutionRequest(selected, session.getRepositorySession());
        request.setResolutionFilter(DependencyFilterUtils.classpathFilter("runtime"));
        var dependencies = session.getContainer().lookup(ProjectDependenciesResolver.class).resolve(request);
        for (var dependency : dependencies.getResolvedDependencies()) {
            var artifact = dependency.getArtifact();
            MavenProject project = projects.get(artifact.getGroupId() + ":" + artifact.getArtifactId() + ":"
                + artifact.getVersion() + ":" + artifact.getExtension() + ":" + artifact.getClassifier());
            if (project != null) {
                classpath.add(Path.of(project.getBuild().getOutputDirectory()).toRealPath());
                modules.add(project.getBasedir().toPath().toRealPath());
            } else {
                if (artifact.getFile() == null) throw new IllegalStateException("Unresolved dependency: " + artifact);
                classpath.add(artifact.getFile().toPath().toRealPath());
            }
        }
        Properties model = new Properties();
        model.setProperty("output", output.toString());
        putPaths(model, "classpath", classpath);
        putPaths(model, "modules", modules);
        try (var stream = Files.newOutputStream(Path.of(destination))) {
            model.store(stream, "Maven resolved runtime model");
        }
    }

    private static String key(Artifact artifact) {
        return artifact.getGroupId() + ":" + artifact.getArtifactId() + ":" + artifact.getVersion()
            + ":" + artifact.getType() + ":" + (artifact.getClassifier() == null ? "" : artifact.getClassifier());
    }

    private static void putPaths(Properties model, String name, LinkedHashSet<Path> paths) {
        model.setProperty(name + ".count", Integer.toString(paths.size()));
        int index = 0;
        for (Path path : paths) model.setProperty(name + "." + index++, path.toString());
    }
}
