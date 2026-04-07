plugins {
    kotlin("jvm")
    id("java-library")
}

group = "io.emqx"
version = "0.4.2"

repositories {
    mavenCentral()
}

dependencies {
    implementation(kotlin("stdlib"))
    implementation("net.java.dev.jna:jna:5.14.0")
}

kotlin {
    jvmToolchain(17)
}
