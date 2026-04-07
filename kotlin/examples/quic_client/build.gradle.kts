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
    mainClass.set("example.quic.MainKt")
}

// Ensure native library is available
tasks.run.configure {
    // The 'package' project copies native library to src/main/resources
    // JNA usually finds it if it's in resource root
}
