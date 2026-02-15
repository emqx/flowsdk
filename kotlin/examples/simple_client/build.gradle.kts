plugins {
    kotlin("jvm")
    id("application")
}

group = "io.emqx.examples"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
    implementation(project(":package"))
    implementation(kotlin("stdlib"))
}

application {
    mainClass.set("example.MainKt")
}

// Ensure native library is available
tasks.run.configure {
    // We need to make sure the native library (dylib/so/dll) is in java.library.path or accessible
    // The 'package' project copies it to src/main/resources, so it should be on classpath.
    // JNA usually finds it if it's in resource root.
}
