plugins {
    kotlin("jvm")
    id("java-library")
}

group = "io.emqx"
version = "0.6.1"

repositories {
    mavenCentral()
}

dependencies {
    implementation(kotlin("stdlib"))
    testImplementation(kotlin("test"))
    implementation("net.java.dev.jna:jna:5.14.0")
}

kotlin {
    jvmToolchain(17)
}

if (providers.gradleProperty("durableSession").orNull == "true") {
    kotlin.sourceSets.named("test") {
        kotlin.srcDir("src/durableTest/kotlin")
    }
}
